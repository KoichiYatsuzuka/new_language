// lexer/editor_tokens.rs — `editor` feature 専用: トークンの**範囲**をエディタへ渡す層。
//
// # なぜ必要か
//
// `Lexer::tokenize()` はトークン列を作るが、`analyze_json` はそれをパーサに渡して
// そのまま捨てていた。結果、VS Code 拡張は着色のために行を
// `/[A-Za-z_]\w*/g` で総なめし、文字列とコメントを自前の `maskLine()` で潰していた
// ＝ **TypeScript 側に 2 つ目の字句解析器**が残っていた。それが本物の lexer とズレる
// （複数行 `"""…"""`、`r"…\"`、`f"{…}"`、`m"…"` / `$…$`）のは構造上避けられない。
//
// ここはその二重実装を消すための最小の追加である。判断は一切しない —
// 「どこからどこまでが 1 トークンか」は `Lexer` がすでに知っているので、それを
// 位置つきで書き出すだけ。
//
// # 通常ビルドでのコスト
//
// モジュールごと `#[cfg(feature = "editor")]`。`Lexer` に足したコメント範囲の
// フィールドも同じ cfg なので、通常ビルドには**存在しない**。
//
// # ⚠ コメントはトークンにならない
//
// `scan.rs` の `Some('#') => { self.skip_comment(); self.next_token() }` が示すとおり、
// コメントは読み飛ばされて `Token::Comment` は存在しない。マスク用途には範囲が要るので、
// `skip_comment()` に記録フックを置いてある（`Lexer::comment_spans`）。
// **`tokenize_with_spans()` の外からは観測できない**ので、ここを消すとコメントの中の
// 識別子が着色されるようになる。

use std::sync::Arc;

use crate::token::{Spanned, Token};

use super::Lexer;

/// トークンの粗い分類。VS Code 側で必要なのはこの粒度だけ。
///
/// 細かく分けないのは、細かくすると「Rust 側の分類」と「拡張側の解釈」を
/// 同期させ続ける必要が生まれ、いま消している問題が別の形で戻るため。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorTokenKind {
    Ident,
    Keyword,
    Str,
    Num,
    Comment,
    Op,
}

impl EditorTokenKind {
    /// JSON へ出すときの名前。拡張側の文字列と一致させること。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ident => "ident",
            Self::Keyword => "keyword",
            Self::Str => "str",
            Self::Num => "num",
            Self::Comment => "comment",
            Self::Op => "op",
        }
    }
}

/// トークン 1 個の範囲。位置は**1 始まりの (行, 列)**、`end` は最終文字の**次**を指す。
///
/// 列は `Span` と同じ「文字単位」。VS Code の UTF-16 単位への変換は
/// `crates/arrow-frontend/src/analyze.rs` が 1 箇所でまとめて行う。
#[derive(Debug, Clone, Copy)]
pub struct EditorTokenSpan {
    pub kind: EditorTokenKind,
    pub start: (usize, usize),
    pub end: (usize, usize),
}

/// `Token` を粗い分類へ落とす。
///
/// キーワードか演算子かは `Token::keyword_str()` に判定させる。ここに一覧を書くと
/// キーワードが増えるたびに手で追随することになり、それは
/// `wasm_providers.ts` 冒頭が「16 個のキーワードが取り残されていた」として
/// 消したはずの失敗そのもの。
fn classify(token: &Token) -> Option<EditorTokenKind> {
    match token {
        // レイアウトトークンは視覚的な幅を持たない。位置も当てにならない
        // （`pending` から返る INDENT/DEDENT は `pos` を動かさない）ので出さない。
        Token::Newline | Token::Indent | Token::Dedent | Token::Eof => None,
        Token::Ident(_) => Some(EditorTokenKind::Ident),
        Token::Str(_) | Token::FStr(_) => Some(EditorTokenKind::Str),
        Token::Int(_) | Token::Float(_) => Some(EditorTokenKind::Num),
        other => Some(if other.keyword_str().is_some() {
            EditorTokenKind::Keyword
        } else {
            EditorTokenKind::Op
        }),
    }
}

/// ソースを 1 度だけ字句解析し、トークン列と**その範囲**の両方を返す。
///
/// パーサに渡すのは第 1 要素。第 2 要素がエディタ用で、コメントも含む
/// （位置順にソート済み）。
///
/// `Lexer::tokenize()` は失敗を表現しないので、**構文エラーでも必ず得られる**。
/// 拡張が「打っている最中にも現在のテキストの色を出せる」根拠がこれ。
pub fn tokenize_with_spans(
    source: &str,
    filename: impl Into<Arc<str>>,
) -> (Vec<Spanned>, Vec<EditorTokenSpan>) {
    let mut lexer = Lexer::new(source, filename);
    let mut tokens: Vec<Spanned> = Vec::new();
    let mut spans: Vec<EditorTokenSpan> = Vec::new();

    loop {
        let before = lexer.pos;
        let spanned = lexer.next_token();
        let after = lexer.pos;
        let done = spanned.token == Token::Eof;

        // 文字を 1 つも消費していないトークン（`pending` 由来の INDENT/DEDENT）は、
        // `after` が「そのトークンの終端」を意味しない。`classify` が None を返す
        // 種別と合わせて二重に弾く。
        if after > before {
            if let Some(kind) = classify(&spanned.token) {
                spans.push(EditorTokenSpan {
                    kind,
                    // 開始は `Spanned` の span を使う（空白を読み飛ばした**後**を指す）。
                    // `before` は読み飛ばす前なので使ってはいけない。
                    start: (spanned.span.line, spanned.span.col),
                    end: lexer.pos_of(after),
                });
            }
        }

        tokens.push(spanned);
        if done {
            break;
        }
    }

    // コメントは `skip_comment()` が別枠で控えている（トークンにならないため）。
    for &(start, end) in lexer.comment_spans() {
        spans.push(EditorTokenSpan { kind: EditorTokenKind::Comment, start, end });
    }
    spans.sort_by_key(|s| s.start);

    (tokens, spans)
}
