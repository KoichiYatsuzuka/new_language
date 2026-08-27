// parser — recursive-descent parser for the Arrow.
// Organized into submodules by role:
//   stmts   — statement parsing (let/mut/const/fn/class/if/for/while/match/...)
//   imports — import/from-import statement parsing and module loading
//   classes — class/trait definition parsing
//   types   — type annotation, template, and parameter parsing
//   exprs   — expression parsing (precedence chain, literals, subscript, ...)

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use crate::ast::{Expr, FieldKind, Stmt, TemplateParam};
use crate::token::{Span, Spanned, Token};

/// `alias Name: rhs` で導入される別名の実体（コンパイル時 AST 置換のペイロード）。
///
/// 別名は純粋な構文置換として振る舞う。使用箇所ごとに右辺の AST/トークンを
/// そのまま挿入するため、`let` と違い評価は毎回行われ、代入対象（lvalue）にもなれる。
///
/// # フィールド
/// - `expr`   : 右辺を式としてパースした AST。式コンテキストでの展開に使う（毎回 clone して差し込む）。
/// - `tokens` : 右辺の生トークン列（末尾に `Eof` 番兵付き）。型注釈コンテキストで
///              `parse_type_expr` により型文字列へ再パースするために使う。
#[derive(Clone)]
pub(crate) struct AliasEntry {
    /// 右辺の式 AST。式位置での置換に用いる。
    pub expr: Rc<Expr>,
    /// 右辺の生トークン列（`Eof` 終端）。型位置での置換（型文字列への再パース）に用いる。
    pub tokens: Rc<Vec<Spanned>>,
}

mod stmts;
// import 解析は 2 実装ある。既定（バッチ実行）はパース時に実モジュールを読み込む
// `imports/`。`editor` feature ではファイルシステム・プロセス・DLL に一切触れない
// `imports_editor.rs` に差し替わる（VS Code 拡張の wasm ビルド用）。
// ⚠ 両者は**同じ構文を受理**しなければならない。詳細は imports_editor.rs の doc。
#[cfg(not(feature = "editor"))]
mod imports;
#[cfg(feature = "editor")]
#[path = "imports_editor.rs"]
mod imports;
mod classes;
mod types;
mod exprs;
// エディタ用の位置情報テーブル（`editor` feature 専用）。AST は変更せず、
// パースの途中で「どの名前がどこにあるか」を控えるだけの副次構造。
#[cfg(feature = "editor")]
pub mod editor_index;
mod editor_hooks;
// .NET アセンブリの読み取り（`import[cs-dll]`）。`editor` では import 自体を
// 構文解釈だけで済ませるので、この重量級モジュールごと外す。
#[cfg(not(feature = "editor"))]
pub(crate) mod cs_assembly;

/// tl 言語の再帰降下パーサ。
///
/// トークン列（`Vec<Spanned>`）を受け取り、プログラム全体の AST（`Vec<Stmt>`）を生成する。
/// import 文の解決・モジュールキャッシュ・循環 import 検出なども担当する。
pub struct Parser {
    tokens: Vec<Spanned>,
    pos: usize,
    /// trait name → (template_params, fields: [(name, kind, type_ann, has_default)], virtual_methods: [name])
    known_traits: HashMap<
        String,
        (
            Vec<TemplateParam>,
            Vec<(String, FieldKind, String, bool)>,
            Vec<String>,
        ),
    >,
    /// Incremented when entering a class/trait body; `Self` is only valid when this is > 0.
    class_or_trait_depth: usize,
    /// Names declared with `new_type` — any reassignment to these is a parse error.
    known_new_types: HashSet<String>,
    /// Names declared with `alias` → substitution payload. Resolved at parse time.
    /// Block-scoped: `parse_block` snapshots and restores this map so an alias declared
    /// inside a block is not visible after the block ends (see `parse_block`).
    aliases: HashMap<String, AliasEntry>,
    /// Names of classes / functions declared with template parameters (`Foo[T: ...]`).
    /// Used to interpret a standalone `Base[Args]` alias RHS as a template instantiation
    /// (rather than a subscript), so `alias X: Base[Arg]` then `X(...)` constructs correctly.
    known_templates: HashSet<String>,
    /// Names declared with `protocol` — instantiation of these is a parse-time error.
    known_protocols: HashSet<String>,
    /// 現在パース中のファイルのディレクトリ（import の第一検索先）。
    // `editor` ではモジュールを読み込まないので、以下 4 つは未使用になる。
    // フィールドごと消さないのは、通常ビルドと `Parser::new` の形を揃えておくため。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    source_dir: PathBuf,
    /// メインエントリーファイルのディレクトリ（import のフォールバック検索先）。
    /// サブパーサにも変更せず引き継がれる。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    root_dir: PathBuf,
    /// モジュールキャッシュ: (lang, 解決済みパス) → 変換済み tl AST。
    /// パース時に同じモジュールを複数回読み込まないために使用する。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    module_cache: HashMap<(String, PathBuf), Vec<Stmt>>,
    /// 循環 import 検出用: 現在読み込み中のモジュールパスのセット。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    loading: HashSet<PathBuf>,
    /// AST 型解決層の node-id 採番カウンタ（タスク #16・段階(a)）。annotatable な Expr を
    /// 構築するたびに `next_node_id()` で採番する。
    ///
    /// **サブパーサと共有してプログラム全体で一意にする**（設計判断 C1「グローバル採番」）。
    /// per-module 採番だと import 先モジュールの node-id がメインと衝突し、
    /// 消費側が**別モジュールの注釈を読んでしまう**。VM の型特化のように実行時フォールバックが
    /// ある消費者は結果が変わらないが、FFI 境界検査のように注釈を信頼する消費者では
    /// **誤検知（正しい値を型不一致と報告）**になる。実際に再現したため共有へ変更した。
    node_counter: std::rc::Rc<std::cell::Cell<u32>>,
    /// エディタ用の位置情報テーブル（`editor` feature 専用・[editor_index] 参照）。
    /// 通常ビルドではフィールドごと存在しない。
    #[cfg(feature = "editor")]
    editor: editor_index::EditorIndex,
}

impl Parser {
    /// パーサを初期化する。
    ///
    /// 組み込みの `Error` トレイトを `known_traits` に事前登録し、
    /// ユーザー定義クラスが `Error` を継承できるようにする。
    ///
    /// # 引数
    /// - `tokens`: レキサが生成したトークン列（`Spanned` の `Vec`）
    /// - `source_dir`: .ar ファイルのディレクトリ（import の第一検索先）
    ///
    /// # 戻り値
    /// 初期化済みの `Parser` インスタンス
    pub fn new(tokens: Vec<Spanned>, source_dir: Option<PathBuf>) -> Self {
        // 組み込み `Error` トレイトを事前登録する。
        // フィールド: message（let・必須）、code_context/file（mut・デフォルトあり）、line/col（mut・デフォルトあり）
        let mut known_traits: HashMap<
            String,
            (
                Vec<TemplateParam>,
                Vec<(String, FieldKind, String, bool)>,
                Vec<String>,
            ),
        > = HashMap::new();
        known_traits.insert(
            "Error".to_string(),
            (
                vec![],
                vec![
                    (
                        "message".to_string(),
                        FieldKind::Let,
                        "str".to_string(),
                        false,
                    ),
                    (
                        "code_context".to_string(),
                        FieldKind::Mut,
                        "str".to_string(),
                        true,
                    ),
                    ("file".to_string(), FieldKind::Mut, "str".to_string(), true),
                    ("line".to_string(), FieldKind::Mut, "int".to_string(), true),
                    ("col".to_string(), FieldKind::Mut, "int".to_string(), true),
                ],
                vec![],
            ),
        );
        let resolved = source_dir.unwrap_or_else(|| PathBuf::from("."));
        Self {
            tokens,
            pos: 0,
            known_traits,
            class_or_trait_depth: 0,
            known_new_types: HashSet::new(),
            aliases: HashMap::new(),
            known_templates: HashSet::new(),
            known_protocols: HashSet::new(),
            source_dir: resolved.clone(),
            root_dir: resolved,
            module_cache: HashMap::new(),
            loading: HashSet::new(),
            node_counter: std::rc::Rc::new(std::cell::Cell::new(0)),
            #[cfg(feature = "editor")]
            editor: editor_index::EditorIndex::new(),
        }
    }

    /// AST 型解決層の node-id を1つ採番する（タスク #16）。1 始まり（0 = 未採番）。
    /// カウンタはサブパーサと共有しているので、プログラム全体で一意になる。
    fn next_node_id(&mut self) -> u32 {
        let next = self.node_counter.get() + 1;
        self.node_counter.set(next);
        // `editor` のときだけ、この node-id が指す式の位置を控える。採番は式を読み終えた
        // 直後に行われるので、直前に消費したトークンがその式の末尾を指す。
        // `Expr::Ident` では末尾＝識別子そのものなので、hover の主用途にはこれで足りる。
        self.note_node_span(next);
        next
    }

    /// 現在位置のトークンへの参照を返す。
    /// トークン列を超えた場合は `Token::Eof` を返す。
    fn current(&self) -> &Token {
        self.tokens
            .get(self.pos)
            .map(|s| &s.token)
            .unwrap_or(&Token::Eof)
    }

    /// 現在位置の1つ先（先読み1トークン）への参照を返す。
    /// トークン列を超えた場合は `Token::Eof` を返す。
    fn peek1(&self) -> &Token {
        self.tokens
            .get(self.pos + 1)
            .map(|s| &s.token)
            .unwrap_or(&Token::Eof)
    }

    /// 現在位置のトークンの `Span`（ファイル名・行・列）を返す。
    /// トークン列を超えた場合は `Span::unknown()` を返す。
    fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|s| s.span.clone())
            .unwrap_or_else(Span::unknown)
    }

    /// 現在位置を1つ進める。トークン列の末尾では何もしない。
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    /// 現在のトークンが `expected` と一致すれば消費して `Ok(())` を返す。
    ///
    /// # エラー
    /// 一致しない場合は「expected `X`, got `Y`」形式のエラー文字列を返す。
    fn eat(&mut self, expected: &Token) -> Result<(), String> {
        if self.current() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected `{}`, got `{}`", expected, self.current()))
        }
    }

    /// 改行・インデント・デデント・セミコロンをまとめてスキップする。
    /// ブロック境界をまたぐ前後のクリーンアップに使用する。
    fn skip_newlines(&mut self) {
        while matches!(
            self.current(),
            Token::Newline | Token::Indent | Token::Dedent | Token::Semicolon
        ) {
            self.advance();
        }
    }
}
