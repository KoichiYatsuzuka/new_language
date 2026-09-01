// python_converter/param_rewrite.rs — `*args` / `**kwargs` のために関数本体の識別子を差し替える仕組み。
//
// Python                          Arrow
// ------------------------------  ---------------------------------------------
// `def f(*xs)` の本体の `xs`       `local::args`（`Expr::LocalVar("args")`）
// `def f(**opts)` の本体の `opts`  `kwargs`（`Expr::Ident("kwargs")`）
//
// Arrow の可変長パラメータは**名前を持たず**（`Param.name` は番兵の `"..."`）、本体からは
// `local::args` で参照する規約。Python 側は任意の名前を付けられるので、**本体の参照を
// 書き換える**しかない。`**kwargs` も同様に、Arrow 側では常に `kwargs` という名前で束縛する。
//
// ⚠ 書き換えを「変換後の AST を歩いて置換する」形にすると `Stmt`/`Expr` 全変種の
//   再帰ウォーカが要る（AST が大きいので割に合わない）。代わりに **変換中に**
//   `convert_expr` の `Name` アームで差し替える。状態は変換器が 1 スレッドで動くことを
//   利用してスレッドローカルのスタックに持つ（`supers.rs` と同じ方式）。

use std::cell::RefCell;
use std::collections::HashMap;

use crate::ast::{Expr, Resolution, PY_KWARGS_PARAM};



/// 本体の識別子をどう差し替えるか。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParamRename {
    /// `local::args`（可変長位置引数）へ。
    LocalArgs,
    /// `kwargs`（可変長キーワード引数）へ。
    Kwargs,
}

thread_local! {
    /// 関数本体ごとの差し替え表。入れ子 `def` のためにスタックにする。
    static RENAMES: RefCell<Vec<HashMap<String, ParamRename>>> =
        const { RefCell::new(Vec::new()) };
}

/// 関数本体の変換中だけ差し替え表を積む RAII ガード。
pub(crate) struct ParamRenameGuard;

impl ParamRenameGuard {
    /// `renames` を積む。
    ///
    /// ⚠ **外側の関数の差し替えを引き継ぐ**（入れ子 `def` から外側の `*args` を参照できる）。
    /// ただし内側の関数が**同じ名前のパラメータを持つ**なら、そちらが勝つので引き継がない
    /// （`own_params` に載っている名前は外側から引き継がない）。
    pub(crate) fn push(renames: HashMap<String, ParamRename>, own_params: &[String]) -> Self {
        RENAMES.with(|s| {
            let mut stack = s.borrow_mut();
            let mut frame = stack.last().cloned().unwrap_or_default();
            frame.retain(|name, _| !own_params.contains(name));
            frame.extend(renames);
            stack.push(frame);
        });
        ParamRenameGuard
    }
}

impl Drop for ParamRenameGuard {
    fn drop(&mut self) {
        RENAMES.with(|s| {
            s.borrow_mut().pop();
        });
    }
}

/// 識別子 `name` に差し替えが登録されていれば、置き換え先の式を返す。
pub(crate) fn renamed_ident(name: &str) -> Option<Expr> {
    RENAMES.with(|s| {
        s.borrow().last().and_then(|frame| {
            frame.get(name).map(|r| match r {
                ParamRename::LocalArgs => Expr::LocalVar("args".to_string()),
                ParamRename::Kwargs => Expr::Ident {
                    name: PY_KWARGS_PARAM.to_string(),
                    node_id: 0,
                    res: Resolution::Unresolved,
                },
            })
        })
    })
}
