// parser/editor_hooks.rs — `editor_index` へ書き込むためのフック。
//
// # 通常ビルドでのコスト
//
// ここのメソッドは本体全体が `#[cfg(feature = "editor")]` で、**通常ビルドでは空関数**になる。
// 引数は `&str` / `Option<&str>` / `usize` だけに絞ってあるので、呼び出し側で
// `String` の確保が起きない ＝ 空関数へのインライン展開で完全に消える。
//
// ⚠ 引数に `String` や `format!` を渡さないこと。渡した瞬間、通常ビルドでも
//    確保コストだけが残る（結果は捨てられるのに）。整形が必要な情報は
//    `#[cfg(feature = "editor")]` を付けた呼び出し側のブロックで組み立てる。

use crate::parser::Parser;

#[cfg(feature = "editor")]
use crate::parser::editor_index::{DeclKind, EditorIndex, Pos};

impl Parser {
    /// 直前に消費したトークンの位置（1 始まりの行・列）。`expect_ident()` の直後に
    /// 呼べば、その識別子そのものを指す。
    ///
    /// `Span` ではなくタプルを返すのは、`Span` が `Arc<str>` を持っていてクローンに
    /// 原子的参照カウントの増減が伴うため。索引にファイル名は要らないので、
    /// 通常ビルドで無駄なコストを残さないよう行・列だけを返す。
    pub(crate) fn prev_pos(&self) -> (usize, usize) {
        self.tokens
            .get(self.pos.saturating_sub(1))
            .map(|s| (s.span.line, s.span.col))
            .unwrap_or((0, 0))
    }

    /// 変数宣言（`let` / `mut` / `const` / `static mut` / `freeze`）を控える。
    ///
    /// ⚠ `expect_ident()` の**直後**に呼ぶこと。位置は `prev_pos()` から取るので、
    ///    型注釈を読んだ後に呼ぶと型の末尾トークンを指してしまう。型注釈は後から
    ///    [`Self::note_type_ann`] でハンドル経由に書き足す。
    #[allow(unused_variables)]
    pub(crate) fn note_var(&mut self, name: &str, mutability: &'static str) -> usize {
        #[cfg(feature = "editor")]
        {
            let pos = self.prev_pos();
            let scope = self.editor.current_scope();
            let mut d = EditorIndex::decl(name.to_string(), DeclKind::Variable, pos, scope);
            d.mutability = Some(mutability);
            return self.editor.push(d);
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// 型注釈を後から書き足す（`let x: int = …` の `int`）。
    #[allow(unused_variables)]
    pub(crate) fn note_type_ann(&mut self, handle: usize, type_ann: Option<&str>) {
        #[cfg(feature = "editor")]
        if let Some(t) = type_ann {
            let t = t.to_string();
            self.editor.with_decl(handle, |d| d.type_ann = Some(t));
        }
    }

    /// 名前付き定義（fn / gen / class / trait / protocol / enum / new_type / alias / module）を控える。
    /// 戻り値は後から `note_signature` などで書き足すためのハンドル（通常ビルドでは 0）。
    #[allow(unused_variables)]
    pub(crate) fn note_def(&mut self, name: &str, kind: EditorKind) -> usize {
        #[cfg(feature = "editor")]
        {
            let pos = self.prev_pos();
            let scope = self.editor.current_scope();
            let container = self.editor.container.clone();
            let mut d = EditorIndex::decl(name.to_string(), kind.into(), pos, scope);
            // クラス本体の中で読まれた fn/gen はそのクラスのメソッド。
            if matches!(kind, EditorKind::Function | EditorKind::Generator) {
                d.container = container;
            }
            return self.editor.push(d);
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// 位置を明示して定義を控える。`prev_pos()` が名前を指していない場面で使う
    /// （例: `import[rs] libm[0.2]` — バージョン括弧を読んだ後は `]` を指してしまう）。
    // 呼び出し元が `imports_editor.rs`（`editor` 限定）だけなので、通常ビルドでは未使用になる。
    #[cfg_attr(not(feature = "editor"), allow(dead_code))]
    #[allow(unused_variables)]
    pub(crate) fn note_def_at(&mut self, name: &str, kind: EditorKind, pos: (usize, usize)) -> usize {
        #[cfg(feature = "editor")]
        {
            let scope = self.editor.current_scope();
            let d = EditorIndex::decl(name.to_string(), kind.into(), pos, scope);
            return self.editor.push(d);
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// 関数の仮引数を控える。**本体スコープが開く前**に呼ばれるので、
    /// 予約領域へ入れて次に開くスコープに配属する。
    ///
    /// 位置は呼び出し側が名前トークン直後に控えたものを渡す（型注釈・デフォルト値を
    /// 読み終えた時点では `prev_pos()` がずれているため）。
    #[allow(unused_variables)]
    pub(crate) fn note_param_at(
        &mut self,
        name: &str,
        mutable: bool,
        type_ann: Option<&str>,
        pos: (usize, usize),
    ) {
        #[cfg(feature = "editor")]
        {
            let scope = self.editor.current_scope();
            let mut d = EditorIndex::decl(name.to_string(), DeclKind::Param, pos, scope);
            d.mutability = Some(if mutable { "mut" } else { "let" });
            d.type_ann = type_ann.map(str::to_string);
            self.editor.push_pending(d);
        }
    }

    /// クラス/トレイトのフィールドを控える。フィールドは本体スコープ内で読まれるので即追加。
    #[allow(unused_variables)]
    pub(crate) fn note_field(&mut self, name: &str, mutability: &'static str) -> usize {
        #[cfg(feature = "editor")]
        {
            let pos = self.prev_pos();
            let scope = self.editor.current_scope();
            let container = self.editor.container.clone();
            let mut d = EditorIndex::decl(name.to_string(), DeclKind::Field, pos, scope);
            d.mutability = Some(mutability);
            d.container = container;
            return self.editor.push(d);
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// enum のメンバを控える。
    #[allow(unused_variables)]
    pub(crate) fn note_enum_member(&mut self, name: &str, container: &str) {
        #[cfg(feature = "editor")]
        {
            let pos = self.prev_pos();
            let scope = self.editor.current_scope();
            let mut d = EditorIndex::decl(name.to_string(), DeclKind::EnumMember, pos, scope);
            d.container = Some(container.to_string());
            self.editor.push(d);
        }
    }

    /// `note_def` / `note_field` が返したハンドルへシグネチャを書き足す。
    #[allow(unused_variables)]
    pub(crate) fn note_signature(&mut self, handle: usize, signature: &str) {
        #[cfg(feature = "editor")]
        self.editor.with_decl(handle, |d| d.signature = Some(signature.to_string()));
    }

    /// docstring を書き足す。
    #[allow(unused_variables)]
    pub(crate) fn note_doc(&mut self, handle: usize, doc: &str) {
        #[cfg(feature = "editor")]
        self.editor.with_decl(handle, |d| d.doc = Some(doc.to_string()));
    }

    /// アクセス制御（`public:` / `private:` / `protected:` セクション）を書き足す。
    #[allow(unused_variables)]
    pub(crate) fn note_access(&mut self, handle: usize, access: &'static str) {
        #[cfg(feature = "editor")]
        self.editor.with_decl(handle, |d| d.access = Some(access));
    }

    /// クラスの継承元トレイト、または関数の仮引数名を書き足す。
    #[allow(unused_variables)]
    pub(crate) fn note_bases(&mut self, handle: usize, bases: &[String]) {
        #[cfg(feature = "editor")]
        self.editor.with_decl(handle, |d| d.bases = bases.to_vec());
    }

    /// この宣言の本体スコープ id を書き足す（fn / class の中身を引くのに使う）。
    #[allow(unused_variables)]
    pub(crate) fn note_body_scope(&mut self, handle: usize, scope: usize) {
        #[cfg(feature = "editor")]
        self.editor.with_decl(handle, |d| d.body_scope = Some(scope));
    }

    /// ブロックスコープを開き、その id を返す。
    #[allow(unused_variables)]
    pub(crate) fn open_editor_scope(&mut self) -> usize {
        #[cfg(feature = "editor")]
        {
            let line = self.current_span().line;
            return self.editor.open_scope(line);
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// ブロックスコープを閉じる。
    pub(crate) fn close_editor_scope(&mut self) {
        #[cfg(feature = "editor")]
        {
            let line = self.prev_pos().0;
            self.editor.close_scope(line);
        }
    }

    /// `next_node_id()` が採番した node-id に、その式の位置を結びつける。
    #[allow(unused_variables)]
    pub(crate) fn note_node_span(&mut self, node_id: u32) {
        #[cfg(feature = "editor")]
        {
            // 採番は式を読み終えた直後に行われるので、直前に消費したトークンが
            // その式の**末尾**を指す。識別子や単純リテラルではそれが名前そのもの。
            let pos: Pos = self.prev_pos();
            if pos.0 != 0 {
                self.editor.node_spans.insert(node_id, pos);
            }
        }
    }

    /// 変数宣言の**初期化式**の node-id を控える。
    ///
    /// inlay hint は「注釈が書かれていない宣言に推論型を出す」機能なので、右辺の型が要る。
    /// 位置から探すやり方は駄目だった: `mut c = Circle(5.0)` の右辺の node 位置は
    /// 式の**末尾**（`)`）を指すため、名前 `c` の近くを探しても見つからない。
    /// node-id を直接控えれば、型検査器の注釈表をそのまま引ける。
    #[allow(unused_variables)]
    pub(crate) fn note_init_expr(&mut self, handle: usize, expr: &crate::ast::Expr) {
        #[cfg(feature = "editor")]
        if let Some(id) = node_id_of(expr) {
            self.editor.with_decl(handle, |d| d.init_node = Some(id));
        }
    }

    /// いま索引に積まれている宣言の件数。クラス本体で「この文が積んだ宣言」を
    /// 後から特定する（アクセス指定の後付けに使う）ための目印。
    pub(crate) fn editor_decl_count(&self) -> usize {
        #[cfg(feature = "editor")]
        {
            return self.editor.decls.len();
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// これから読むメンバ宣言が属するクラス/トレイト名を設定する。
    #[allow(unused_variables)]
    pub(crate) fn set_editor_container(&mut self, name: &str) {
        #[cfg(feature = "editor")]
        {
            self.editor.container = Some(name.to_string());
        }
    }

    /// クラス本体を抜けたので `container` を解除する。
    pub(crate) fn clear_editor_container(&mut self) {
        #[cfg(feature = "editor")]
        {
            self.editor.container = None;
        }
    }

    /// 次に `open_editor_scope()` が割り当てるスコープ id。
    /// 本体をパースする前に控えておき、`note_body_scope` に渡す。
    pub(crate) fn next_editor_scope_id(&self) -> usize {
        #[cfg(feature = "editor")]
        {
            return self.editor.scopes.len();
        }
        #[cfg(not(feature = "editor"))]
        0
    }

    /// 収集した索引を取り出す（`parse_program` の後に呼ぶ）。
    #[cfg(feature = "editor")]
    pub fn editor_index(&self) -> &crate::parser::editor_index::EditorIndex {
        &self.editor
    }
}

/// `note_def` に渡す種別。`editor` が無効なときも呼び出し側が同じコードで書けるように、
/// `DeclKind` とは別の（常に存在する）列挙にしてある。
// `Module` は `imports_editor.rs`（`editor` 限定）からしか作られないので、
// 通常ビルドでは未構築のバリアントになる。
#[cfg_attr(not(feature = "editor"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorKind {
    Function,
    Generator,
    Class,
    Trait,
    Protocol,
    Enum,
    NewType,
    Alias,
    Module,
}

#[cfg(feature = "editor")]
impl From<EditorKind> for DeclKind {
    fn from(k: EditorKind) -> Self {
        match k {
            EditorKind::Function => DeclKind::Function,
            EditorKind::Generator => DeclKind::Generator,
            EditorKind::Class => DeclKind::Class,
            EditorKind::Trait => DeclKind::Trait,
            EditorKind::Protocol => DeclKind::Protocol,
            EditorKind::Enum => DeclKind::Enum,
            EditorKind::NewType => DeclKind::NewType,
            EditorKind::Alias => DeclKind::Alias,
            EditorKind::Module => DeclKind::Module,
        }
    }
}

/// 式が持つ node-id（型解決層の索引キー）。採番されていない式は `None`。
#[cfg(feature = "editor")]
fn node_id_of(expr: &crate::ast::Expr) -> Option<u32> {
    use crate::ast::Expr;
    let id = match expr {
        Expr::Ident { node_id, .. }
        | Expr::Attr { node_id, .. }
        | Expr::BinOp { node_id, .. }
        | Expr::Call { node_id, .. }
        | Expr::Subscript { node_id, .. }
        | Expr::Cast { node_id, .. } => *node_id,
        _ => 0,
    };
    (id != 0).then_some(id)
}

/// 関数・クラスの本体先頭にある docstring（文字列リテラル 1 個の文）を取り出す。
pub(crate) fn docstring_of(body: &[crate::ast::Stmt]) -> Option<&str> {
    match body.first() {
        Some(crate::ast::Stmt::Expr(crate::ast::Expr::Str(s))) => Some(s),
        _ => None,
    }
}

/// hover に出す関数シグネチャ文字列を組み立てる。
///
/// 例: `fn translate(self, let dx: int, let dy: int) -> Point`
///
/// ⚠ `#[cfg(feature = "editor")]` を付けた呼び出し側からのみ使うこと。通常ビルドでも
///    呼べてしまうが、`format!` の確保が実行経路に残ってしまう。
#[cfg(feature = "editor")]
pub(crate) fn render_fn_signature(
    keyword: &str,
    name: &str,
    params: &[crate::ast::Param],
    return_type: Option<&str>,
) -> String {
    let rendered: Vec<String> = params
        .iter()
        .map(|p| {
            if p.name == "self" {
                // `mut self` は「このメソッドが自身を書き換える」重要な情報なので落とさない。
                return if p.mutable { "mut self".to_string() } else { "self".to_string() };
            }
            let qualifier = if p.mutable { "mut" } else { "let" };
            let name = if p.variadic { "..." } else { p.name.as_str() };
            match &p.type_ann {
                Some(t) => format!("{qualifier} {name}: {t}"),
                None => format!("{qualifier} {name}"),
            }
        })
        .collect();
    match return_type {
        Some(rt) => format!("{keyword} {name}({}) -> {rt}", rendered.join(", ")),
        None => format!("{keyword} {name}({})", rendered.join(", ")),
    }
}
