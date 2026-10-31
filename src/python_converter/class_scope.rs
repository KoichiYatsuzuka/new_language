// python_converter/class_scope.rs — クラス本体の名前を参照するクラス属性（python_builtins_plan.md のタスク 6-3）。
//
// Python のクラス本体は 1 つの名前空間で、`x = property(getx, setx)` / `DOUBLE = LIMIT * 2` / `table = {"get": getx}`
// のように、先に定義したメソッドやクラス属性を素の名前で引ける。Arrow の `static mut` の初期化式からは
// クラス本体の名前もクラス自身も見えない（`NameError`）。そこで変換器は、そうした初期化式を
// **クラスの定義の後で** `C.x = property(C.getx, C.setx)` と組み立てる（6-1 の `@property` と同じ考え方）。
// ここはその「素の名前 → `C.名前`」の書き換え。
//
// ⚠ Python の名前の規則に合わせ、クラス本体の名前が見えるのは**クラス本体の式そのもの**だけ:
//   - ラムダの本体は見えない（`f = lambda: LIMIT` の `LIMIT` は大域を引く）
//   - 内包表記は**最初の `for` の反復対象**だけが見える（`[i for i in range(N)]` の `N` は見える、要素の式は見えない）

use std::collections::HashSet;
use std::convert::Infallible;

use rustpython_parser::ast as py;
use rustpython_parser::ast::fold::Fold;
use rustpython_parser::text_size::TextRange;

/// `names`（その時点までにクラス本体で束縛された名前）の読みを `<class_name>.<name>` に書き換える。
pub(crate) struct ClassNameRewriter<'a> {
    names: &'a HashSet<String>,
    class_name: &'a str,
    /// 1 つでも書き換えたか。
    pub(crate) hit: bool,
}

impl<'a> ClassNameRewriter<'a> {
    /// `expr` を書き換えて、書き換えたかどうかと一緒に返す。
    pub(crate) fn rewrite(expr: py::Expr, names: &'a HashSet<String>, class_name: &'a str) -> (py::Expr, bool) {
        let mut r = ClassNameRewriter { names, class_name, hit: false };
        let Ok(out) = r.fold_expr(expr);
        (out, r.hit)
    }

    /// 内包表記の生成子のうち、最初の反復対象だけを書き換える（Python の名前の規則）。
    fn fold_first_iter(&mut self, mut gens: Vec<py::Comprehension>) -> Vec<py::Comprehension> {
        if gens.is_empty() {
            return gens;
        }
        let first = gens.remove(0);
        let Ok(iter) = self.fold_expr(first.iter);
        gens.insert(0, py::Comprehension { iter, ..first });
        gens
    }
}

impl Fold<TextRange> for ClassNameRewriter<'_> {
    type TargetU = TextRange;
    type Error = Infallible;
    type UserContext = ();

    fn will_map_user(&mut self, _user: &TextRange) -> Self::UserContext {}

    fn map_user(&mut self, user: TextRange, _context: Self::UserContext) -> Result<TextRange, Infallible> {
        Ok(user)
    }

    fn fold_expr(&mut self, node: py::Expr) -> Result<py::Expr, Infallible> {
        match node {
            py::Expr::Name(n) if matches!(n.ctx, py::ExprContext::Load) && self.names.contains(n.id.as_str()) => {
                self.hit = true;
                Ok(py::Expr::Attribute(py::ExprAttribute {
                    range: n.range,
                    value: Box::new(py::Expr::Name(py::ExprName {
                        range: n.range,
                        id: py::Identifier::new(self.class_name),
                        ctx: py::ExprContext::Load,
                    })),
                    attr: n.id,
                    ctx: py::ExprContext::Load,
                }))
            }
            // ラムダの本体からはクラス本体の名前が見えない（既定値の式は見えるが、それは珍しいので書き換えない）。
            py::Expr::Lambda(_) => Ok(node),
            py::Expr::ListComp(c) => {
                let generators = self.fold_first_iter(c.generators);
                Ok(py::Expr::ListComp(py::ExprListComp { generators, ..c }))
            }
            py::Expr::SetComp(c) => {
                let generators = self.fold_first_iter(c.generators);
                Ok(py::Expr::SetComp(py::ExprSetComp { generators, ..c }))
            }
            py::Expr::GeneratorExp(c) => {
                let generators = self.fold_first_iter(c.generators);
                Ok(py::Expr::GeneratorExp(py::ExprGeneratorExp { generators, ..c }))
            }
            py::Expr::DictComp(c) => {
                let generators = self.fold_first_iter(c.generators);
                Ok(py::Expr::DictComp(py::ExprDictComp { generators, ..c }))
            }
            other => py::fold::fold_expr(self, other),
        }
    }
}
