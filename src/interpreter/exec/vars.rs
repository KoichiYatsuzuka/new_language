// exec/vars.rs — 変数宣言・代入の実行: let / タプル束縛 / static / 複合代入 / loop_yield とスロットキャッシュ。

use {
    std::cell::RefCell,
    std::rc::Rc,
    crate::ast::{
        BinOp, Expr, TupleTarget,
    },
    crate::token::Span,
    crate::interpreter::{
        ExecResult,
        Interpreter, Value, Var,
    },
};

/// 宣言の型注釈が `float` で値が `Int` のとき `Float` へ昇格する（案 B・2026-09-08）。
///
/// # なぜ要るのか
///
/// 注釈が `float` でも値が `Int` のまま束縛されるため、**注釈が嘘をつく**状態が実在した:
/// ```text
/// let b: float = 3   → 3    （昇格しない）
/// k.x = 7（x: float） → 7.0  （昇格する）
/// ```
/// 後者だけ昇格しているのは型検査の結果ではなく、`store_field` の raw レイアウト経路に
/// `int → float フィールドの自動昇格` アームがあるという**メモリレイアウトの副作用**。
/// 受理側は `type_matches` の `(Int, Float)` アームで広げたので、値の側もここで揃える。
///
/// ⚠⚠ **副作用として VM の型特化が正しくなる。** [`crate::vm`] の `slot_prim` は
/// `slot_type`（AST に書かれた注釈）を見て int/float 特化 op を選ぶが、昇格前は
/// `let b: float = 3` が「注釈は float・実値は Int」だったため、選ばれた float 特化 op が
/// 毎回汎用へフォールバックしていた。
///
/// ⚠ **スカラの `float` 注釈だけを対象にする。** `list[float] = [1, 2]` のような
/// 要素単位の昇格は行わない（要素ごとの走査になり、`let` の複製規則とも絡む）。
/// 拡大は `int` → `float` の一方向のみで、逆（`float` → `int`）は情報を落とすので行わない。
pub(crate) fn coerce_binding(type_ann: Option<&str>, value: Value) -> Value {
    let Some(ann) = type_ann else { return value };
    // C ABI 別名（`float32` / `float64`）も基底型で判定する。
    let ann = crate::ast::c_abi_base_type(ann).unwrap_or(ann);
    match (ann, &value) {
        ("float", Value::Int(n)) => Value::Float(*n as f64),
        _ => value,
    }
}

/// 宣言の束縛で値をどう複製するか（native_object_access_plan.md 1-6）。
///
/// 束縛の複製の**唯一の実装** [`Interpreter::bind_copy`] を選ぶ札。消費者はツリーウォークの
/// `exec_let` / `Stmt::Mut`、VM の `StoreLocal*` / `StoreCellDeepCopy` / `DeclareGlobal`、
/// ネイティブの `ar_bind_value`（生成コードが `i32` の符号で渡す・[`BindCopy::from_code`]）。
/// ⚠ 以前は同じ複製が 6 箇所に書かれており、L2（式からの `let` は常に複製）を直したとき
///   VM の `StoreLocalFreezeInstance` だけが `Instance` しか複製しないまま残った（関数の中の
///   `let it = ys[0]` が `ys[0]` と共有された・§0.3 ⑦）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BindCopy {
    /// `mut x = e`: 常に深いコピー
    DeepCopy = 0,
    /// `let x = <mut の名前>` / `let x = <識別子以外の式>`: 深いコピーしてフリーズ（L2）
    CopyFreeze = 1,
}

impl BindCopy {
    /// 生成コードが符号に足す bit: 束縛する値は、コールバックが作りたての値を積んだばかりの欄
    /// （`ar_bind_value`・生成コードの `fresh_regs`・12-1）。[`Self::from_code`] には外してから渡す。
    pub(crate) const FRESH_BIT: i32 = 2;

    /// 生成コードが渡す符号（`as i32` の逆）。表に無い符号は `None`（呼ぶ側がエラーにする）。
    pub(crate) fn from_code(code: i32) -> Option<Self> {
        match code {
            0 => Some(Self::DeepCopy),
            1 => Some(Self::CopyFreeze),
            _ => None,
        }
    }
}

impl Interpreter {
    /// 束縛の複製（[`BindCopy`]）。`CopyFreeze` のフリーズは `__freeze__` を呼びうるので失敗する。
    /// 誰とも共有していない値（[`Interpreter::is_unshared`]・作りたての `Vec3(...)` など）は複製しない（12-1・D-13）。
    /// 写しを作っても、元を指す者がいないので違いは観測できない（`deep_copy_value` はフリーズの印も写す）。
    /// ⚠ 数えるのは `Rc` の参照なので、値を持つ者が全員参照を数える場所（VM のスタック・欄・ツリーウォーク）だけで使う。
    ///   ネイティブのアリーナはローカル変数が番号で同じ欄を指すので数えられない（`bind_copy_held`）。
    pub(crate) fn bind_copy(&mut self, mode: BindCopy, value: Value) -> Result<Value, String> {
        self.bind_copy_held(mode, value, Some(0))
    }

    /// [`Self::bind_copy`] の本体。`held` は、呼ぶ側が `value` のほかに持っていて数えない参照の数（ネイティブの
    /// アリーナの、作りたての値を積んだ欄 1 つなど）。`None` なら共有かどうかを見ずに必ず複製する。
    pub(crate) fn bind_copy_held(&mut self, mode: BindCopy, value: Value, held: Option<usize>) -> Result<Value, String> {
        let unshared = held.is_some_and(|h| Self::is_unshared(&value, 1 + h));
        let copied = if unshared { value } else { Self::deep_copy_value(value) };
        if mode == BindCopy::CopyFreeze {
            self.apply_freeze_to_value(&copied, true)?;
        }
        Ok(copied)
    }

    /// `let` 宣言を実行する。
    pub(crate) fn exec_let(
        &mut self,
        name: &str,
        type_ann: Option<&str>,
        expr: &Expr,
    ) -> Result<ExecResult, String> {
        if name != "_" && self.get_var(name).is_some() {
            return Err(format!("NameError: variable '{name}' is already declared"));
        }
        // mut → let: deep copy してからフリーズする。
        // let → let: そのまま代入（コピー不要・再フリーズ不要）。
        // 式 → let: Instance の場合は deep copy してからフリーズする。
        //   可変コレクション (list[i] など) から取り出した Instance を直接フリーズすると
        //   共有 Rc を通じて元のオブジェクトまで不変化されてしまうため、コピーが必要。
        let source_mutable = if let Expr::Ident { name: src, .. } = expr {
            self.get_var(src).map(|v| v.is_mutable())
        } else {
            None
        };
        let value = self.eval(expr)?;
        // mut の名前 → 複製してフリーズ・let の名前 → そのまま・識別子以外の式（`xs[0]` / `o.f` / `f()` /
        // リテラル）→ 複製してフリーズ。VM の `StoreLocalFromIdent` / `DeclareGlobal` と同じ実装を呼ぶ。
        //
        // ⚠⚠ 識別子以外の式は**常に複製する**（L2）。以前は `Instance` のときだけ複製しており、
        //    コレクションは共有されていた。実測での実害:
        //      mut xs = [[1]]; let item = xs[0]; xs[0].append(9)
        //      → **`let` の `item` が `[1, 9]` に変わる**
        //    リテラルも同じで、`let L = [p]` は `p` を共有していた。
        // ⚠ 式には引き継ぐべき属性が無いので `let → let` の共有には当たらない。
        //    ⇒ 「共有するのは `let → let` のときだけ」の規則どおり複製する。
        let value = self.vm_let_value_from_ident(source_mutable, value)?;
        // 案 B: `float` 注釈なら昇格する（VM の `Op::CoerceFloat` と同じ位置・同じ判断）。
        // ⚠ 複製・フリーズの**後**に置く。昇格後の `Float` はプリミティブなので順序に
        //    意味は無いが、`coerce_binding` を「ストア直前」に固定しておくと VM 側と
        //    位置が揃い、片方だけ動かしたときに差が出にくい。
        let value = coerce_binding(type_ann, value);
        self.declare_var(name.to_string(), Var::new(value, false));
        Ok(ExecResult::Normal)
    }

    /// タプル分解を伴う `let (a, b, ...) = expr` 宣言を実行する。
    pub(crate) fn exec_let_tuple(
        &mut self,
        targets: &[TupleTarget],
        value: &Expr,
    ) -> Result<ExecResult, String> {
        let val = self.eval(value)?;
        self.exec_let_tuple_evaled(targets, val)
    }

    /// 評価済みの値でタプル分解宣言を行う（最上位 VM の `Op::LetTuple` 用・#27-c）。
    /// 束縛先が現スコープ（＝グローバル）の場合。
    pub(crate) fn exec_let_tuple_evaled(
        &mut self,
        targets: &[TupleTarget],
        val: Value,
    ) -> Result<ExecResult, String> {
        for target in targets.iter() {
            match target {
                TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => {
                    if n != "_" && self.get_var(n).is_some() {
                        return Err(format!("NameError: variable '{n}' is already declared"));
                    }
                }
                TupleTarget::Wildcard => {}
            }
        }
        for (i, v) in self.let_tuple_values(targets, val)? {
            let mutable = matches!(targets[i], TupleTarget::Mut(_));
            let name = match &targets[i] {
                TupleTarget::Let(n) | TupleTarget::Bare(n) | TupleTarget::Mut(n) => n.clone(),
                TupleTarget::Wildcard => continue,
            };
            self.declare_var(name, Var::new(v, mutable));
        }
        Ok(ExecResult::Normal)
    }

    /// タプル分解の**検査と値の取り出し**（#27-c）。
    ///
    /// ツリーウォーク（`exec_let_tuple`）と VM（`Op::LetTuple` の両経路）の**唯一の実装**。
    /// 型検査・要素数検査・エラー文言・`let` の freeze / `mut` の deep_copy がここに集約されている。
    /// 戻り値は `(targets の index, 束縛する値)` を宣言順に並べたもの（`Wildcard` 以降は打ち切り）。
    ///
    /// ⚠ 「既に宣言済み」検査は**ここに入れない**。最上位はスコープに宣言するので必要だが、
    /// VM のフラット slot には宣言集合が無く、ツリーウォークも反復ごとにスコープを push し直す
    /// ため入れ子では発生しない。束縛先を知っている呼び出し側の責務。
    pub(crate) fn let_tuple_values(
        &mut self,
        targets: &[TupleTarget],
        val: Value,
    ) -> Result<Vec<(usize, Value)>, String> {
        let tuple_rc = match val {
            Value::Tuple(rc) => rc,
            _ => {
                return Err(
                    "TypeError: cannot unpack non-tuple value in tuple assignment".to_string(),
                )
            }
        };
        let has_wildcard = targets.iter().any(|t| matches!(t, TupleTarget::Wildcard));
        let named = targets
            .iter()
            .filter(|t| !matches!(t, TupleTarget::Wildcard))
            .count();
        let tlen = tuple_rc.len();
        if has_wildcard {
            if named > tlen {
                return Err(format!(
                    "TypeError: not enough values to unpack (need at least {named}, got {tlen})"
                ));
            }
        } else if named != tlen {
            return Err(format!(
                "TypeError: not enough values to unpack (expected {named}, got {tlen})"
            ));
        }
        let mut out = Vec::with_capacity(named);
        let mut idx = 0usize;
        for (i, target) in targets.iter().enumerate() {
            match target {
                TupleTarget::Wildcard => break,
                TupleTarget::Let(_) | TupleTarget::Bare(_) => {
                    let v = tuple_rc.get(idx).unwrap().clone();
                    self.apply_freeze_to_value(&v, false)?;
                    out.push((i, v));
                    idx += 1;
                }
                TupleTarget::Mut(_) => {
                    let v = Self::deep_copy_value(tuple_rc.get(idx).unwrap().clone());
                    out.push((i, v));
                    idx += 1;
                }
            }
        }
        Ok(out)
    }

    /// `static mut` 変数宣言を実行する。ソース位置をキーに静的セルを確保し、呼び出し間で値を共有する。
    pub(crate) fn exec_static_var(
        &mut self,
        name: &str,
        expr: &Expr,
        span: &Span,
    ) -> Result<ExecResult, String> {
        let key = (span.file.to_string(), span.line, span.col);
        let cell = if let Some(existing) = self.static_cells.get(&key) {
            existing.clone()
        } else {
            let value = self.eval(expr)?;
            let new_cell = Rc::new(RefCell::new(value));
            self.static_cells.insert(key, new_cell.clone());
            new_cell
        };
        self.declare_var(name.to_string(), Var::new_cell(cell));
        Ok(ExecResult::Normal)
    }

    /// `+=` / `-=` などの複合代入文を実行する。変数の可変性を確認してから演算結果を書き戻す。
    pub(crate) fn exec_compound_assign(
        &mut self,
        name: &str,
        op: &BinOp,
        value: &Expr,
        slot: &crate::ast::SlotCache,
    ) -> Result<ExecResult, String> {
        // スロットキャッシュ命中: 読み・書きともスコープ検索なしの直接セルアクセス
        if let Some(idx) = slot.get(self.slot_epoch) {
            let rhs = self.eval(value)?;
            let cell = self.global_slot_cells[idx].clone();
            let lhs = cell.borrow().clone();
            let result = self.apply_binop_dyn(op, lhs, rhs)?;
            *cell.borrow_mut() = result;
            return Ok(ExecResult::Normal);
        }
        let rhs = self.eval(value)?;
        let lhs = match self.get_var(name) {
            Some(v) if !v.is_mutable() => {
                return Err(format!(
                    "TypeError: cannot assign to immutable variable '{name}'"
                ));
            }
            Some(v) => v.get_value(),
            None => return Err(format!("NameError: '{name}' is not defined")),
        };
        let value = self.apply_binop_dyn(op, lhs, rhs)?;
        self.assign_var(name, value)?;
        self.try_fill_slot(name, slot);
        Ok(ExecResult::Normal)
    }

    /// 代入文のスロットキャッシュ充填を試みる（変数のスロット化）。
    ///
    /// 条件: `name` がローカルスコープ（1..）に存在せず、グローバルスコープ（0）の
    /// 可変変数として解決されること。`Var::Mutable` は `Var::SlotCell` に昇格し、
    /// セルを `global_slot_cells` レジストリに登録してインデックスを AST に焼き込む。
    ///
    /// 健全性: グローバルは再宣言禁止のためバインディングは固定。`freeze` は
    /// `make_var_immutable` が `SlotCell` を `Immutable` に降格させ `slot_epoch` を
    /// 進めるので、全キャッシュが自動失効する。
    pub(crate) fn try_fill_slot(&mut self, name: &str, slot: &crate::ast::SlotCache) {
        // 現関数のローカル（frame_floor..）に同名があればグローバル解決ではない
        if self.scopes[self.frame_floor..].iter().any(|s| s.contains_key(name)) {
            return;
        }
        let Some(var) = self.scopes[0].get_mut(name) else {
            return;
        };
        let cell = match var {
            Var::SlotCell(rc) | Var::Cell(rc) => rc.clone(),
            Var::Mutable(v) => {
                let rc = Rc::new(RefCell::new(std::mem::replace(v, Value::None)));
                *var = Var::SlotCell(rc.clone());
                rc
            }
            Var::Immutable(_) => return,
        };
        let idx = self.global_slot_cells.len();
        if idx >= (u32::MAX - 1) as usize {
            return;
        }
        self.global_slot_cells.push(cell);
        slot.fill(self.slot_epoch, idx as u32);
    }

    // ---------------------------------------------------------------------------
    // Control flow signals
    // ---------------------------------------------------------------------------


    // ---------------------------------------------------------------------------
    // Control flow structures
    // ---------------------------------------------------------------------------

}
