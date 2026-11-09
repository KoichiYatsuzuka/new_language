// vm/compiler/visibility.rs — ブロックスコープの見え方（name_scope_plan.md の R-1）。
//
// Arrow の束縛はブロックスコープ（`docs/grammar/02_variables.md`）。`if` / `while` / `for` / `match` /
// `block:` / `try` / `except` の本体で宣言した名前は、そのブロックを抜けると見えない。宣言より前からも見えない。
//
// ⚠⚠ 枠（slot）は関数全体で 1 つの平らな表で、本体の宣言を入れ子のブロックまで**先に全部**集めて振る
//   （`collect_nested_decls`）。枠の初期値は `Value::None` で、`LoadLocal` は書かれたかどうかを見ない。
//   以前はこのため、ブロックの後の読みが中の値を読み（`if True: let y = 1` の後の `y` が `1`）、一度も
//   実行されなかった `let` の読みが `None` になり、ループの本体で宣言より前の読みが**前の周回の値**を読んだ。
//   最上位の文はスコープを積んで実行するので `NameError` になっていた（関数の中だけ違った）。
//
// ⇒ コンパイル時に「今見えている名前」を持つ。宣言の文をコンパイルし終えたら見えるようにし、宣言のある
//   ブロックを抜けたら外す。本体が宣言する名前（`block_locals`）のうち見えていないものを読み書きしたら、
//   その場で `Op::Fail` を出して `NameError` にする（B4 の `for` 変数の解放と同じ形）。
//   宣言はブロックの頭から順に実行されるので、ソースの位置で決めた見え方が実行時の見え方と一致する
//   （`if False:` の中の宣言も、その後ろからは見えない）。**実行時の負担は無い**（正しいプログラムの
//   バイトコードは変わらない。`Op::Fail` が出るのは誤った読み書きだけ）。
//
// ⚠ Python のモジュールの関数（`Chunk::is_python`）には当てない（`block_locals` が空）。Python は関数単位の
//   スコープで、`if` の中で代入した名前を後で読むのが正しい（`import[py]` の関数はこの形に頼っている）。

use crate::ast::Stmt;
use crate::vm::op::Op;

use super::Compiler;

impl Compiler {
    /// 見え方を管理する名前（この Chunk の本体が宣言する名前）を登録する。
    ///
    /// ⚠ 入口ごとに「宣言を集めた直後の `slots`」から引数・キャプチャを除いたものを渡す。
    /// ⚠ Python のモジュールの関数・文では呼ばない（モジュールの doc）。
    pub(super) fn track_block_locals(&mut self, names: impl IntoIterator<Item = String>) {
        self.block_locals.extend(names);
    }

    /// 本体が宣言する名前で、今は見えていないか（読み書きすると `NameError`）。
    #[inline]
    pub(super) fn is_unbound_local(&self, name: &str) -> bool {
        self.block_locals.contains(name) && !self.visible_locals.contains(name)
    }

    /// `name` を今のブロックの終わりまで見えるようにする。
    pub(super) fn make_visible(&mut self, name: &str) {
        if self.block_locals.contains(name) && self.visible_locals.insert(name.to_string()) {
            if let Some(scope) = self.block_scopes.last_mut() {
                scope.push(name.to_string());
            }
        }
    }

    /// 文 `stmt` が直接束縛する名前を見えるようにする（文をコンパイルし終えた後に呼ぶ）。
    ///
    /// ⚠ 束縛の列挙は [`crate::decl_names::each_declared_name`] 1 箇所（枠を振る `collect_nested_decls` と同じ）。
    ///   `for` の変数・`except` の別名は文が束縛するのではなく本体のブロックの中で見えるので、それぞれ
    ///   `compile_for` / `compile_try_except` が本体のブロックの頭で見えるようにする。
    pub(super) fn make_declared_visible(&mut self, stmt: &Stmt) {
        if self.block_locals.is_empty() {
            return;
        }
        let mut names: Vec<String> = Vec::new();
        crate::decl_names::each_declared_name(stmt, &mut |name, _, _| names.push(name.to_string()));
        for n in names {
            self.make_visible(&n);
        }
    }

    /// ブロックに入る。
    pub(super) fn enter_block(&mut self) {
        self.block_scopes.push(Vec::new());
    }

    /// ブロックを抜ける。このブロックで見えるようになった名前を外す。
    pub(super) fn leave_block(&mut self) {
        // ⚠ 先頭（本体そのもの）は抜けない。`enter_block` と対にしか呼ばないので来ないはず。
        if self.block_scopes.len() > 1 {
            for n in self.block_scopes.pop().unwrap_or_default() {
                self.visible_locals.remove(&n);
            }
        }
    }

    /// ブロックの本体 `stmts` を、新しいブロックとしてコンパイルする。
    ///
    /// ⚠ 本体の文を回すループは**これを通す**こと（`for s in body { self.compile_stmt(s)?; }` と直接書くと、
    ///   その本体の宣言がブロックの外まで見えたままになる）。途中で失敗してもブロックは抜ける。
    pub(super) fn compile_block_body(&mut self, stmts: &[Stmt]) -> Option<()> {
        self.enter_block();
        let r = (|| {
            for s in stmts {
                self.compile_stmt(s)?;
            }
            Some(())
        })();
        self.leave_block();
        r
    }

    /// `for` の本体（文・式の両方）。ループ変数 `targets` は本体のブロックの中でだけ見える。
    ///
    /// ⚠ ループ変数はブロックの**頭で**見えるようにする（`iter` の式の中からは見えない）。
    ///   ブロックを抜けた後の読みは、従来どおり `released_for_targets` も `Op::Fail` にする（B4）。
    pub(super) fn compile_loop_body(&mut self, targets: &[String], body: &[Stmt]) -> Option<()> {
        self.enter_block();
        for t in targets {
            self.make_visible(t);
        }
        let r = (|| {
            for s in body {
                self.compile_stmt(s)?;
            }
            Some(())
        })();
        self.leave_block();
        r
    }

    /// `except E as e:` のハンドラ本体。別名 `alias` はハンドラのブロックの中でだけ見える。
    pub(super) fn compile_handler_body(&mut self, alias: Option<&String>, body: &[Stmt]) -> Option<()> {
        self.enter_block();
        if let Some(a) = alias {
            self.make_visible(a);
        }
        let r = (|| {
            for s in body {
                self.compile_stmt(s)?;
            }
            Some(())
        })();
        self.leave_block();
        r
    }

    /// 見えていない名前 `name` の読み書き。最上位（スコープを積むツリーウォーク）と同じ文言で落とす。
    pub(super) fn emit_unbound_local(&mut self, name: &str) {
        let n = self.add_name(&format!("NameError: '{name}' is not defined"));
        self.emit(Op::Fail(n));
    }
}
