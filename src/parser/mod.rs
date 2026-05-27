// parser — recursive-descent parser for the tl language.
// Organized into submodules by role:
//   stmts   — statement parsing (let/mut/const/fn/class/if/for/while/match/...)
//   imports — import/from-import statement parsing and module loading
//   classes — class/trait definition parsing
//   types   — type annotation, template, and parameter parsing
//   exprs   — expression parsing (precedence chain, literals, subscript, ...)

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ast::{FieldKind, Stmt, TemplateParam};
use crate::token::{Span, Spanned, Token};

mod stmts;
mod imports;
mod classes;
mod types;
mod exprs;

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
    /// 現在パース中のファイルのディレクトリ（import の第一検索先）。
    source_dir: PathBuf,
    /// メインエントリーファイルのディレクトリ（import のフォールバック検索先）。
    /// サブパーサにも変更せず引き継がれる。
    root_dir: PathBuf,
    /// モジュールキャッシュ: (lang, 解決済みパス) → 変換済み tl AST。
    /// パース時に同じモジュールを複数回読み込まないために使用する。
    module_cache: HashMap<(String, PathBuf), Vec<Stmt>>,
    /// 循環 import 検出用: 現在読み込み中のモジュールパスのセット。
    loading: HashSet<PathBuf>,
}

impl Parser {
    /// パーサを初期化する。
    ///
    /// 組み込みの `Error` トレイトを `known_traits` に事前登録し、
    /// ユーザー定義クラスが `Error` を継承できるようにする。
    ///
    /// # 引数
    /// - `tokens`: レキサが生成したトークン列（`Spanned` の `Vec`）
    /// - `source_dir`: .tl ファイルのディレクトリ（import の第一検索先）
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
            source_dir: resolved.clone(),
            root_dir: resolved,
            module_cache: HashMap::new(),
            loading: HashSet::new(),
        }
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
