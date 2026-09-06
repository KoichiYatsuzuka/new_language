// parser/editor_index.rs — `editor` feature 専用: エディタが必要とする位置情報の副次テーブル。
//
// # なぜ AST を変更しないのか
//
// hover / inlay / go-to-definition / semantic tokens は「カーソル位置 → その名前は何か」
// の逆引きを必要とする。AST に span を足せば済むが、`Stmt::Let` のようなタプルバリアントに
// フィールドを 1 つ足すだけで **237 箇所以上**（インタプリタ・VM コンパイラ・LLVM codegen・
// テンプレート置換・AST リフレクション）に波及する。実行経路を巻き込む変更は
// 「エディタを直す」という目的に対して代償が大きすぎる。
//
// 代わりに、**パーサが宣言を読んだその場で位置を控える**副次テーブルを持つ。AST は 1 バイトも
// 変わらないので、インタプリタ・VM・codegen への影響はゼロ。通常ビルドではこのモジュールごと
// 存在しない（`#[cfg(feature = "editor")]`）。
//
// # スコープ
//
// `parse_block` / `parse_class_body` が唯一のブロック境界なので、そこで open/close する。
// 関数の仮引数は本体ブロックが開く**前**に読まれるため、`pending` に溜めて次に開いた
// スコープへ流し込む（そうしないと引数が呼び出し側のスコープに見えてしまう）。

use std::collections::HashMap;

/// ソース位置（1 始まりの行・列）。`Span` を使わないのは `Arc<str>` の
/// クローンを避けるため — ファイル名はパース単位で一定なので索引には要らない。
pub type Pos = (usize, usize);

/// 宣言の種別。VS Code の CompletionItemKind / SymbolKind へ拡張側で対応付ける。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Variable,
    Param,
    Field,
    Function,
    Generator,
    Class,
    Trait,
    Protocol,
    Enum,
    EnumMember,
    NewType,
    Alias,
    Module,
}

impl DeclKind {
    /// JSON へ出すときの名前。拡張側の `DeclKind` 文字列と一致させること。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Variable => "variable",
            Self::Param => "param",
            Self::Field => "field",
            Self::Function => "function",
            Self::Generator => "generator",
            Self::Class => "class",
            Self::Trait => "trait",
            Self::Protocol => "protocol",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::NewType => "new_type",
            Self::Alias => "alias",
            Self::Module => "module",
        }
    }
}

/// 1 つの宣言。`span` は**名前トークン**の位置（キーワードではなく）。
#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub kind: DeclKind,
    pub pos: Pos,
    /// `let` / `mut` / `const` / `static mut` / `freeze`。宣言でないものは `None`。
    pub mutability: Option<&'static str>,
    /// ソースに書かれた型注釈。推論結果ではない（推論型は型検査器の注釈から取る）。
    pub type_ann: Option<String>,
    /// 関数なら `fn name(a: int) -> str` 形式の完全シグネチャ。
    pub signature: Option<String>,
    /// docstring（本体先頭の文字列リテラル）。
    pub doc: Option<String>,
    /// `public` / `private` / `protected`。クラス本体の外では `None`。
    pub access: Option<&'static str>,
    /// このメンバを保持するクラス/トレイト名。トップレベル宣言は `None`。
    pub container: Option<String>,
    /// クラスの継承元トレイト、または関数の仮引数名リスト。
    pub bases: Vec<String>,
    /// この宣言が**見えるようになる**スコープ。
    pub scope: usize,
    /// 関数・クラスの本体スコープ（`None` は本体を持たない宣言）。
    pub body_scope: Option<usize>,
    /// 変数宣言の初期化式の node-id。型注釈が無いときの推論型はここから引く。
    pub init_node: Option<u32>,
}

/// 字句的スコープ。`end_line` は閉じるまで `usize::MAX`。
#[derive(Debug, Clone)]
pub struct Scope {
    pub parent: Option<usize>,
    pub start_line: usize,
    pub end_line: usize,
}

/// パース中に集める位置情報一式。
#[derive(Debug, Default)]
pub struct EditorIndex {
    pub decls: Vec<Decl>,
    pub scopes: Vec<Scope>,
    /// `Expr` の node-id → その式の位置。型検査器の `AstAnnotations`（node-id → 推論型）と
    /// 突き合わせると「カーソル下の式の型」が引ける。
    pub node_spans: HashMap<u32, Pos>,
    /// **型注釈位置に現れた型名**とその位置（`let x: int` の `int`）。
    ///
    /// これが無いと、拡張は識別子を**名前だけ**で宣言表に引くことになり、`int` のように
    /// 型名と組み込み関数を兼ねる 9 個の名前
    /// （`bool` / `float` / `int` / `path` / `set` / `slice` / `str` / `type` / `uint`）が
    /// すべて関数として着色・hover される。型位置を知っているのは
    /// [`Parser::parse_type_expr`](crate::parser::Parser) だけなので、そこで控えるしかない。
    pub type_refs: Vec<(Pos, String)>,
    /// alias 展開の入れ子深さ。0 より大きいあいだは型参照を記録しない（[`Self::push_type_ref`]）。
    alias_depth: usize,
    /// 構文エラーで停止したトークンの位置（1 始まりの (行, 列)）。
    ///
    /// # なぜ索引に持つのか
    ///
    /// `parse_program` は `Result<_, String>` を返す ＝ **位置を人間向けの文字列に埋めて捨てて**
    /// いる。拡張はそれを正規表現で読み直して波線の位置を決めていた。
    /// エラーの型を変えれば根治するが、66 箇所の `Err(format!(…))` と全呼び出し元、
    /// そして端末出力（`compare_outputs.ps1` の比較対象）に波及する。
    ///
    /// 位置だけが要るのだから、宣言や型参照と同じく**副次テーブルに控える**のが釣り合う。
    /// パーサが止まった時点の `self.pos` がそのまま失敗位置なので、推測は一切要らない。
    pub parse_error_pos: Option<Pos>,
    /// 現在のスコープ id。
    current: usize,
    /// 次に開くスコープへ入れる宣言（関数の仮引数・クラスのフィールド）。
    pending: Vec<Decl>,
    /// いまパース中のクラス/トレイト/プロトコル名。メンバ宣言の `container` に入れる。
    pub container: Option<String>,
}

impl EditorIndex {
    /// ルートスコープ 1 つを持った状態で作る。
    pub fn new() -> Self {
        Self {
            decls: Vec::new(),
            scopes: vec![Scope { parent: None, start_line: 0, end_line: usize::MAX }],
            node_spans: HashMap::new(),
            type_refs: Vec::new(),
            alias_depth: 0,
            parse_error_pos: None,
            current: 0,
            pending: Vec::new(),
            container: None,
        }
    }

    /// 現在のスコープ id。
    pub fn current_scope(&self) -> usize {
        self.current
    }

    /// 新しい子スコープを開き、その id を返す。`pending` の宣言はここへ流し込む。
    pub fn open_scope(&mut self, start_line: usize) -> usize {
        let id = self.scopes.len();
        self.scopes.push(Scope {
            parent: Some(self.current),
            start_line,
            end_line: usize::MAX,
        });
        self.current = id;
        for mut d in std::mem::take(&mut self.pending) {
            d.scope = id;
            self.decls.push(d);
        }
        id
    }

    /// 現在のスコープを閉じて親へ戻る。
    pub fn close_scope(&mut self, end_line: usize) {
        self.scopes[self.current].end_line = end_line;
        if let Some(parent) = self.scopes[self.current].parent {
            self.current = parent;
        }
        // 本体が無かった等で流し込まれなかった pending は捨てる（漏らすと誤ったスコープに出る）。
        self.pending.clear();
    }

    /// 型注釈位置に現れた型名を控える。位置不明（`line == 0`）と重複は捨てる。
    ///
    /// alias 展開中（[`Self::enter_alias`]）は**何もしない**。展開中に見える位置は
    /// alias の**定義側**トークンのものなので、そのまま記録すると使用箇所と無関係な行に
    /// 型色が付く（import 経由の alias なら他ファイルの行番号が紛れ込む）。
    pub fn push_type_ref(&mut self, pos: Pos, name: &str) {
        if self.alias_depth > 0 || pos.0 == 0 {
            return;
        }
        if self.type_refs.iter().any(|(p, _)| *p == pos) {
            return;
        }
        self.type_refs.push((pos, name.to_string()));
    }

    /// alias 展開に入る（型参照の記録を止める）。
    pub fn enter_alias(&mut self) {
        self.alias_depth += 1;
    }

    /// alias 展開から抜ける。
    pub fn leave_alias(&mut self) {
        self.alias_depth = self.alias_depth.saturating_sub(1);
    }

    /// 宣言を現在のスコープに追加し、そのインデックスを返す。
    pub fn push(&mut self, decl: Decl) -> usize {
        let idx = self.decls.len();
        self.decls.push(decl);
        idx
    }

    /// 宣言を「次に開くスコープ」へ予約する（関数の仮引数など）。
    /// インデックスは確定しないので返さない。
    pub fn push_pending(&mut self, decl: Decl) {
        self.pending.push(decl);
    }

    /// `push` で得たインデックスの宣言を書き換える。存在しなければ何もしない。
    pub fn with_decl(&mut self, idx: usize, f: impl FnOnce(&mut Decl)) {
        if let Some(d) = self.decls.get_mut(idx) {
            f(d);
        }
    }

    /// 空の `Decl` を作る補助。呼び出し側は必要なフィールドだけ埋める。
    pub fn decl(name: String, kind: DeclKind, pos: Pos, scope: usize) -> Decl {
        Decl {
            name,
            kind,
            pos,
            mutability: None,
            type_ann: None,
            signature: None,
            doc: None,
            access: None,
            container: None,
            bases: Vec::new(),
            scope,
            body_scope: None,
            init_node: None,
        }
    }
}
