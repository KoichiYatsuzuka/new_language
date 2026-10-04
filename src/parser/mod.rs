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
// モジュール指定（`..a.b`）の構文。上の 2 実装が**共有**する（受理する構文をずらさないため）。
mod import_syntax;
pub(crate) mod classes;
mod types;
// 型の文字列を型注釈と同じ綴りに揃える（展開時の型の値・D23 / タスク 4-2）。
pub(crate) use types::canonical_type_text;
mod exprs;
// Python ソースからの行ベース型スタブ抽出。**両ビルドで使う**ので `imports/` の外に置く
// （`imports/` は editor では丸ごと差し替わり、抽出器ごと消えてしまうため）。
pub(crate) mod py_stub_extract;
// ホストが渡した型スタブの表（`editor` 専用）。CLI は実モジュールを読むので要らない。
#[cfg(feature = "editor")]
pub mod stub_registry;
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
/// `trait` 宣言のうち、**後続のクラス定義が必要とする情報**だけを抜き出したもの。
///
/// ⚠ 元は 3 要素タプルだったが、デフォルト実装（`default_methods`）を運ぶ必要が出たので
/// 名前付きにした。要素が増えたときタプルの位置で覚える必要がなくなる。
#[derive(Clone)]
pub(crate) struct TraitInfo {
    pub(crate) template_params: Vec<TemplateParam>,
    /// `(name, kind, type_ann, has_default)`
    pub(crate) fields: Vec<(String, FieldKind, String, bool)>,
    /// **本体が `...` の仮想メソッド**名。クラスは必ず override しなければならない
    /// （`collect_trait_fields_and_check_virtuals` が検査する）。
    pub(crate) virtual_methods: Vec<String>,
    /// **本体を持つメソッド（デフォルト実装）。** クラスが同名を定義していなければ
    /// クラス本体へ注入する（`inject_trait_default_methods`）。
    ///
    /// ⚠ 注入は**パース時**に行う。`exec_trait_def` は trait のメソッド本体を保持しないので
    /// 実行時に引き継ぐ先が無く、これが無いと「宣言はできるが呼べない」状態になっていた。
    pub(crate) default_methods: Vec<Stmt>,
}

pub struct Parser {
    /// ⚠ `Rc` で持つ（タスク 4-7）。宣言の `SrcRange` がファイル 1 本を**共有**するため。
    /// 別名展開（`expand_alias_as_type`）は一時的に差し替えるので、宣言の範囲は
    /// こちらではなく [`Parser::main_tokens`] を指す。
    tokens: std::rc::Rc<Vec<Spanned>>,
    /// このパーサが読んでいる**ファイル本体の**トークン列（タスク 4-7）。差し替えない。
    ///
    /// ⚠ 別名展開中は `tokens` が別名の右辺へ差し替わる。宣言の範囲をそちらで取ると
    /// **別のトークン列を切り出す**ので、必ずこちらを使う。
    main_tokens: std::rc::Rc<Vec<Spanned>>,
    pos: usize,
    /// trait 名 → その trait の宣言情報（[`TraitInfo`]）。
    known_traits: HashMap<String, TraitInfo>,
    /// Incremented when entering a class/trait body; `Self` is only valid when this is > 0.
    class_or_trait_depth: usize,
    /// メタ関数の本体に入ると増える。`^`（メタ情報演算子）が書ける文脈の片方
    /// （設計書 §1.5 / タスク 1-3）。
    pub(crate) metafn_depth: usize,
    /// 入れ子のブロック（関数・`if` / `for` などの本体・クラス本体）の深さ（フェーズ10 10-16）。
    /// 0 のときだけ `class` / `trait` / `protocol` / `new_type` / `import` を書ける。
    pub(crate) block_depth: usize,
    /// 呼び出しの実引数リストをパース中なら増える。`^` が書ける文脈のもう片方。
    ///
    /// ⚠ **これは近似**。本来の規則は「**メタ関数**呼び出しの実引数位置」だが、
    /// パース時には呼び先がメタ関数かどうか分からない（解決は展開器の仕事）。
    /// ⇒ ここでは「呼び出しの実引数の中」まで緩めて受け、**正しいコードを決して弾かない**
    /// 側に倒す。呼び先がメタ関数かの検査は展開器（Phase 2）が行う。
    pub(crate) call_arg_depth: usize,
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
    /// 現在パース中のファイルのディレクトリ（相対 import の起点・[`crate::module_path`]）。
    // `editor` ではモジュールを読み込まないので、以下 5 つは未使用になる。
    // フィールドごと消さないのは、通常ビルドと `Parser::new` の形を揃えておくため。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    source_dir: PathBuf,
    /// `source_dir` が**ファイルのディレクトリとして与えられたか**（`Parser::new` の `Some`）。
    ///
    /// ⚠ 与えられなかった（REPL・テスト・メタ関数の部分ソース）ときは、import 文に
    ///   探索の起点を**載せない**（`ImportOrigin::base_dir` が `None`）。実行時はそのとき
    ///   登録済みの探索先（`Interpreter::python_search_dirs`）を使う。`.`（CWD）を起点として
    ///   載せると、テストが登録した探索先が使われなくなる。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    has_source_dir: bool,
    /// メインエントリーファイルのディレクトリ。サブパーサにも変更せず引き継がれる。
    ///
    /// **ドット無しの import の探索先**（CPython の `sys.path[0]`・[`crate::module_path`]）で、
    /// **モジュールの名前の基準**（`pkg.util`・[`crate::module_path::root_relative_name`]）でもある。
    /// ⚠ 2026-10-02 午前の版（フェーズ 1）では探索先から外していたが、CPython 準拠（フェーズ 4）で
    ///   ドット無しの import の唯一の探索先（言語ごとの外部の探索先を除く）になった。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    root_dir: PathBuf,
    /// モジュールキャッシュ: (lang, 解決済みパス) → 変換済み tl AST。
    /// パース時に同じモジュールを複数回読み込まないために使用する。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    module_cache: HashMap<(String, PathBuf), Vec<Stmt>>,
    /// 循環 import 検出用: 現在読み込み中のモジュールパスのセット。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    loading: HashSet<PathBuf>,
    /// ファイル ↔ モジュール名の対応表（[`crate::module_path::ModuleNames`]）。
    /// **サブパーサと共有する**（プログラム全体で 1 つ。`node_counter` と同じ扱い）。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    module_names: std::rc::Rc<std::cell::RefCell<crate::module_path::ModuleNames>>,
    /// 今の文の**前に**置く文（CPython 準拠・2026-10-02）。`import a.b.c` は先にパッケージ `a` と
    /// `a.b` を読み込む（束縛しない `Stmt::Import` を足す）。`parse_program` が最上位の文の前へ並べる。
    /// ⚠ import は最上位にしか書けない（10-16）ので、溜めるのも出すのも最上位だけ。
    #[cfg_attr(feature = "editor", allow(dead_code))]
    pending_stmts: Vec<Stmt>,
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
        let mut known_traits: HashMap<String, TraitInfo> = HashMap::new();
        known_traits.insert(
            "Error".to_string(),
            TraitInfo {
                template_params: vec![],
                virtual_methods: vec![],
                default_methods: vec![],
                fields: vec![
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
            },
        );
        let has_source_dir = source_dir.is_some();
        let resolved = source_dir.unwrap_or_else(|| PathBuf::from("."));
        // ⚠⚠ **テンプレート名はパースを始める前に全部集める**（タスク 9.7）。
        //    以前は `parse_class_def` が到達した時点で 1 つずつ登録していたので、
        //    **宣言より前に書いた注釈では型引数が黙って捨てられていた**（実測）:
        //      fn use_it(b: Box[int]) -> int:   # ← `[int]` が消え `Box` になる
        //      class Box[T]: ...                #   （宣言はこの後）
        //    さらに `trait X[T]` は**登録すらされていなかった**（class だけ登録していた）。
        let known_templates = Self::scan_template_names(&tokens);
        let tokens_rc = std::rc::Rc::new(tokens);
        Self {
            tokens: std::rc::Rc::clone(&tokens_rc),
            main_tokens: tokens_rc,
            pos: 0,
            known_traits,
            class_or_trait_depth: 0,
            metafn_depth: 0,
            block_depth: 0,
            call_arg_depth: 0,
            known_new_types: HashSet::new(),
            aliases: HashMap::new(),
            known_templates,
            known_protocols: HashSet::new(),
            source_dir: resolved.clone(),
            has_source_dir,
            root_dir: resolved,
            module_cache: HashMap::new(),
            loading: HashSet::new(),
            module_names: std::rc::Rc::default(),
            pending_stmts: Vec::new(),
            node_counter: std::rc::Rc::new(std::cell::Cell::new(0)),
            #[cfg(feature = "editor")]
            editor: editor_index::EditorIndex::new(),
        }
    }

    /// トークン列を 1 度走査して `class X[...]` / `trait X[...]` の **X** を集める
    /// （タスク 9.7）。
    ///
    /// ⚠ **パース順に依存しないことがこの関数の存在理由。** 型注釈は宣言より前に
    /// 現れうるので、逐次登録では「そのときまだ知らない」テンプレートが出る。
    /// ⚠ 走査するのは自分のトークン列だけ。import 先のテンプレートは含まれない
    /// （別ファイルは別 `Parser` が読む）。それらの `Base[Args]` は
    /// [`Self::parse_type_expr`] が「型引数を取らない型」として弾く。
    fn scan_template_names(tokens: &[Spanned]) -> HashSet<String> {
        let mut set = HashSet::new();
        for w in tokens.windows(3) {
            if matches!(w[0].token, Token::Class | Token::Trait)
                && matches!(w[2].token, Token::LBracket)
            {
                if let Token::Ident(n) = &w[1].token {
                    set.insert(n.clone());
                }
            }
        }
        set
    }

    /// 宣言に元のソースの範囲を付ける（タスク 4-7）。`start` は宣言の先頭位置。
    ///
    /// ⚠ 付けるのは**まだ付いていない**ものだけ。`!装飾子` の中の宣言は内側の
    /// 呼び出しで既に付いていて、外側（装飾子行を含む範囲）で上書きしてはいけない
    /// ——`.code()` が装飾子行ごと返すと、置き直したときに装飾子が二重にかかる。
    /// 文に**文の位置**（先頭のトークンの位置）を付ける（フェーズ10 10-17・[`crate::ast::Stmt::position`]）。
    ///
    /// ⚠ `parse_stmt` の入口の 1 箇所で付ける（各文の構文解析は位置を空で作る）。
    pub(crate) fn attach_position(&self, stmt: &mut crate::ast::Stmt, start: usize) {
        if let Some(t) = self.tokens.get(start) {
            stmt.fill_position(&t.span);
        }
    }

    pub(crate) fn attach_src(&self, stmt: &mut crate::ast::Stmt, start: usize) {
        use crate::ast::{SrcRange, Stmt};
        let range = || {
            Some(SrcRange { tokens: std::rc::Rc::clone(&self.main_tokens), start, end: self.pos })
        };
        match stmt {
            Stmt::FnDef { src, .. }
            | Stmt::GenDef { src, .. }
            | Stmt::ClassDef { src, .. }
            | Stmt::TraitDef { src, .. }
            | Stmt::EnumDef { src, .. }
            | Stmt::Field { src, .. } => {
                if src.is_none() {
                    *src = range();
                }
            }
            _ => {}
        }
    }

    /// この `Parser` が使っている node-id カウンタ（設計書 §0.3 / タスク 2-1）。
    ///
    /// ⚠⚠ **node-id はプログラム全体で一意**でなければならない。per-module 採番にしたら
    /// **別モジュールの注釈を読んでしまい、FFI 境界検査で誤検知が実際に再現した**
    /// （`node_counter` の doc）。⇒ 展開器が置いたコードも**同じカウンタから採番する**。
    pub(crate) fn node_counter(&self) -> std::rc::Rc<std::cell::Cell<u32>> {
        std::rc::Rc::clone(&self.node_counter)
    }

    /// パース中に集めた trait の宣言情報（タスク 2-4）。
    ///
    /// ⚠ 展開器がクラス本体の仕上げ（`finalize_class_body`）を走らせるのに要る。
    /// 装飾子でメンバーが増えたクラスは、**展開後に**自動 `__init__` 生成と
    /// trait デフォルト実装の注入をやり直す必要がある。
    pub(crate) fn known_traits(&self) -> std::collections::HashMap<String, TraitInfo> {
        self.known_traits.clone()
    }

    /// node-id カウンタを差し替える（設計書 §0.3 / タスク 2-1）。
    ///
    /// ⚠ **パースを始める前に呼ぶこと。** 途中で差し替えると採番が飛ぶ。
    /// ⚠ import のサブパーサが `sub.node_counter = self.node_counter.clone()` としているのと
    /// 同じことを、`src/parser/` の外（展開器）からできるようにしたもの。
    pub(crate) fn set_node_counter(&mut self, counter: std::rc::Rc<std::cell::Cell<u32>>) {
        self.node_counter = counter;
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
    /// 現在位置の `Spanned` トークンを複製して返す（`code:` ブロックが
    /// 中身を**パースせずトークン列のまま**保持するために使う・メタ関数 1-1）。
    pub(crate) fn spanned_at_pos(&self) -> crate::token::Spanned {
        self.tokens[self.pos].clone()
    }

    pub(crate) fn current_span(&self) -> Span {
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
