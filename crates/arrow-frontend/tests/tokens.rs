//! `analyze_json` が出す `tokens` の回帰テスト。
//!
//! # 何を守っているか
//!
//! 拡張はこの配列だけを見て着色・語の特定・受け手判定を行う（自前の正規表現走査と
//! `maskLine()` は撤去済み）。つまりここが崩れると拡張の言語機能が丸ごと崩れる。
//!
//! テストの主眼は、**撤去した `maskLine()` が間違えていたケース**が本物の lexer では
//! 正しいことを固定すること — 複数行 `"""…"""`、raw 文字列の `\`、f-string の補間、
//! `m"…"` / `$…$` 数学文字列、そして構文エラー中でもトークンが出ること。
//!
//! ⚠ ルートの `cargo test` では走らない（このクレートはワークスペースから exclude）。
//!    `cd crates/arrow-frontend && cargo test` で実行すること。

use arrow_frontend::analyze::analyze_json;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
struct Tok {
    kind: String,
    line: u64,
    col: u64,
    end_line: u64,
    end_col: u64,
}

fn tokens(source: &str) -> Vec<Tok> {
    let raw = analyze_json(source, "test.ar");
    let v: Value = serde_json::from_str(&raw).expect("valid JSON");
    v["tokens"]
        .as_array()
        .expect("tokens must be an array")
        .iter()
        .map(|t| Tok {
            kind: t["kind"].as_str().unwrap().to_string(),
            line: t["line"].as_u64().unwrap(),
            col: t["col"].as_u64().unwrap(),
            end_line: t["endLine"].as_u64().unwrap(),
            end_col: t["endCol"].as_u64().unwrap(),
        })
        .collect()
}

/// その位置を覆うトークンを返す（拡張の `tokenAt` と同じ判定）。
fn covering(toks: &[Tok], line: u64, col: u64) -> Option<&Tok> {
    toks.iter().find(|t| {
        (t.line < line || (t.line == line && t.col <= col))
            && (t.end_line > line || (t.end_line == line && t.end_col > col))
    })
}

/// 基本形。識別子・キーワード・演算子・数値が種別つきで出る。
#[test]
fn basic_kinds() {
    let toks = tokens("let a = 5\n");
    assert_eq!(covering(&toks, 0, 0).map(|t| t.kind.as_str()), Some("keyword"), "let — {toks:?}");
    assert_eq!(covering(&toks, 0, 4).map(|t| t.kind.as_str()), Some("ident"), "a — {toks:?}");
    assert_eq!(covering(&toks, 0, 6).map(|t| t.kind.as_str()), Some("op"), "= — {toks:?}");
    assert_eq!(covering(&toks, 0, 8).map(|t| t.kind.as_str()), Some("num"), "5 — {toks:?}");

    // 識別子の範囲はちょうど名前の長さ。
    let a = covering(&toks, 0, 4).unwrap();
    assert_eq!((a.line, a.col, a.end_line, a.end_col), (0, 4, 0, 5));
}

/// コメントは**トークンにならない**ので別枠で拾っている。落ちると
/// コメント内の識別子が着色される。
#[test]
fn comments_are_reported() {
    let toks = tokens("let a = 5  # note: int here\n");
    let c = covering(&toks, 0, 13).expect("comment must be covered — {toks:?}");
    assert_eq!(c.kind, "comment", "{toks:?}");
    assert_eq!((c.line, c.col), (0, 11), "コメントは # から始まる — {toks:?}");

    // コメント内の `int` がトークンとして出てはいけない。
    assert_eq!(covering(&toks, 0, 19).map(|t| t.kind.as_str()), Some("comment"), "{toks:?}");
}

/// 複数行 `"""…"""`。旧 `maskLine()` は行単位で状態を持たないため、
/// docstring の 2 行目以降の識別子を着色していた。
#[test]
fn multiline_triple_quoted_string_spans_lines() {
    let src = "fn f() -> None:\n\
               \x20   \"\"\"doc line one\n\
               int str float should NOT be tokens\n\
               \"\"\"\n\
               \x20   pass\n";
    let toks = tokens(src);

    // 2 行目の `int` は文字列の内側。
    let inside = covering(&toks, 2, 0).expect("line 2 must be covered by the string — {toks:?}");
    assert_eq!(inside.kind, "str", "{toks:?}");
    assert_eq!(inside.line, 1, "文字列は 1 行目から始まる — {toks:?}");
    assert!(inside.end_line >= 3, "文字列は 3 行目まで続く — {toks:?}");
}

/// 文字列のプレフィックス（`r` / `f` / `m` / `b` / `fr` / `rf`）は**文字列トークンの一部**。
///
/// 旧 `maskLine()` は引用符しか見ないのでプレフィックス文字が裸で残り、識別子走査が
/// `r` という名前の識別子を拾っていた（同名の変数があればそこに着色・hover が付く）。
#[test]
fn string_prefix_belongs_to_the_string_token() {
    let toks = tokens("let p = r\"abc\"\nlet q = b\"xy\"\n");

    let r = covering(&toks, 0, 8).expect("prefix `r` — {toks:?}");
    assert_eq!(r.kind, "str", "プレフィックスは文字列の一部 — {toks:?}");
    assert_eq!(r.col, 8, "文字列は `r` から始まる — {toks:?}");

    let b = covering(&toks, 1, 8).expect("prefix `b` — {toks:?}");
    assert_eq!(b.kind, "str", "{toks:?}");
}

/// raw 文字列でも `\` は次の 1 文字を巻き込む（`src/lexer/literal.rs` の raw 分岐）。
/// つまり `r"a\"` は閉じず、そこから先はすべて文字列になる。
///
/// ⚠ 撤去した `maskLine()` も同じ扱いだったので、ここは**ズレていなかった**。
///   「raw だから `\` はただの文字」と思い込んで直すと、lexer と食い違う側に倒れる。
#[test]
fn raw_string_backslash_still_consumes_next_char() {
    let toks = tokens("let p = r\"a\\\" + b\nlet q = 1\n");
    let s = covering(&toks, 0, 8).expect("string — {toks:?}");
    assert_eq!(s.kind, "str", "{toks:?}");
    assert!(s.end_line > 0, "閉じないので次行以降まで伸びる — {toks:?}");
    // 2 行目の `1` も文字列の内側。
    assert_eq!(covering(&toks, 1, 8).map(|t| t.kind.as_str()), Some("str"), "{toks:?}");
}

/// f-string は 1 トークン。補間の中身まで別トークンに割れたりしない。
#[test]
fn fstring_is_one_token() {
    let toks = tokens("let n = 1\nlet s = f\"v={n + 1}\"\n");
    let f = covering(&toks, 1, 8).expect("f-string — {toks:?}");
    assert_eq!(f.kind, "str", "{toks:?}");
    // 補間の `n` の位置も同じ文字列トークンに覆われている。
    assert_eq!(covering(&toks, 1, 13).map(|t| t.kind.as_str()), Some("str"), "{toks:?}");
}

/// 数学文字列 `m"…"` / `$…$`。旧 `maskLine()` は存在自体を知らなかった。
#[test]
fn math_strings_are_strings() {
    let toks = tokens("let a = m\"\\alpha\"\nlet b = $x^2$\n");
    assert_eq!(covering(&toks, 0, 8).map(|t| t.kind.as_str()), Some("str"), "m\"…\" — {toks:?}");
    assert_eq!(covering(&toks, 1, 8).map(|t| t.kind.as_str()), Some("str"), "$…$ — {toks:?}");
}

/// 列は **UTF-16 コードユニット**。サロゲートペアの後ろで 1 ずれてはいけない。
#[test]
fn columns_are_utf16() {
    //                0123456789...
    let src = "let a = \"\u{1F600}\" + b\n";
    let toks = tokens(&src);

    // 文字単位だと `+` は 12、`b` は 14。絵文字が UTF-16 で 2 単位なので +1 ずれる。
    assert_eq!(covering(&toks, 0, 13).map(|t| t.kind.as_str()), Some("op"), "+ — {toks:?}");
    let b = covering(&toks, 0, 15).expect("b — {toks:?}");
    assert_eq!(b.kind, "ident", "{toks:?}");
    assert_eq!(b.col, 15, "UTF-16 列になっていない — {toks:?}");
}

/// 構文エラー中でもトークンは**現在のテキストのもの**が出る。
///
/// これが無いと拡張は lastGood（古いテキスト）の位置を今のバッファに当てることになり、
/// 打っている最中ずっと色がずれる。
#[test]
fn tokens_survive_parse_error() {
    let raw = analyze_json("let a: int =\nlet b = 2\n", "test.ar");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["ok"], Value::Bool(false), "この入力は構文エラーのはず");

    let toks = tokens("let a: int =\nlet b = 2\n");
    assert!(!toks.is_empty(), "構文エラーでもトークンは出る");
    assert_eq!(covering(&toks, 1, 4).map(|t| t.kind.as_str()), Some("ident"), "2 行目の b — {toks:?}");
}

/// 構文エラーの位置は**パーサが止まったトークン**として出る。
///
/// 以前は拡張がエラーメッセージ本文に正規表現を当てて行・列を読み直していた。
/// メッセージの書き方を変えると静かに壊れる依存だったので、索引から取るようにした。
#[test]
fn parse_error_position_comes_from_the_parser() {
    // 2 行目の `=` の直後で式が来ずに改行 → そこで止まる。
    let raw = analyze_json("let a = 1\nlet b: int =\n", "test.ar");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["ok"], Value::Bool(false), "この入力は構文エラーのはず");

    let at = &v["parseErrorAt"];
    assert!(!at.is_null(), "位置が出ていない: {raw}");
    assert_eq!(at["line"].as_u64(), Some(1), "2 行目で止まるはず — {at}");
}

/// 構文が通るときは `parseErrorAt` は null。
#[test]
fn parse_error_position_is_null_when_ok() {
    let raw = analyze_json("let a = 1\n", "test.ar");
    let v: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(v["ok"], Value::Bool(true));
    assert!(v["parseErrorAt"].is_null(), "{raw}");
}

/// レイアウトトークン（NEWLINE / INDENT / DEDENT / EOF）は出さない。
/// 幅を持たないうえ `pending` 由来のものは位置が当てにならない。
#[test]
fn no_zero_width_layout_tokens() {
    let toks = tokens("fn f() -> None:\n    pass\n");
    for t in &toks {
        assert!(
            t.end_line > t.line || t.end_col > t.col,
            "幅ゼロのトークンが出ている: {t:?}",
        );
    }
}
