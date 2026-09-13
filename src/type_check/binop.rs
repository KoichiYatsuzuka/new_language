use crate::ast::BinOp;
use crate::token::Span;

use super::errors::{StaticTypeError, TypeErrorKind};
use super::types::InferredType;
use super::TypeChecker;

impl TypeChecker {
    /// 二項演算子の型検査を行い、`Any` 型・`Union` 型への演算や順序比較の不整合をエラーとして記録する。
    pub(super) fn check_binop(
        &mut self,
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
        span: Span,
    ) {
        if *lt == InferredType::Any || *rt == InferredType::Any {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnAny {
                    op: op.as_str().to_string(),
                },
                span: Some(span),
            });
            return;
        }
        let union_side = if matches!(lt, InferredType::Union(_)) {
            Some(lt)
        } else if matches!(rt, InferredType::Union(_)) {
            Some(rt)
        } else {
            None
        };
        if let Some(union_ty) = union_side {
            self.report_error(StaticTypeError {
                kind: TypeErrorKind::OperationOnUnion {
                    union_type: union_ty.to_string(),
                    op: op.as_str().to_string(),
                },
                span: Some(span),
            });
            return;
        }
        match op {
            BinOp::Lt => self.check_ordered_cmp(lt, rt, "<", span),
            BinOp::Gt => self.check_ordered_cmp(lt, rt, ">", span),
            BinOp::LtEq => self.check_ordered_cmp(lt, rt, "<=", span),
            BinOp::GtEq => self.check_ordered_cmp(lt, rt, ">=", span),
            // ── 算術・ビット演算の可否（Kind 3・タスク 4.4）──────────────────
            //
            // ⚠⚠ **以前は順序比較だけ検査していた。** `1 + "s"` のような最も頻出の型エラーが
            //    静的に出ず、実行時の `unsupported operand types` まで判らなかった。
            // ⚠ 判定は `infer_binop_result` が `Unresolved` を返すかで行う（D-9: 同じ関数が
            //    可否と結果型の両方を答える）。**2 つの表を持つとずれる**ので 1 本に寄せた。
            BinOp::Add
            | BinOp::Sub
            | BinOp::Mul
            | BinOp::Div
            | BinOp::FloorDiv
            | BinOp::Mod
            | BinOp::Pow
            | BinOp::BitAnd
            | BinOp::BitOr
            | BinOp::BitXor
            | BinOp::LShift
            | BinOp::RShift => self.check_arith(op, lt, rt, span),
            // ── `==` / `!=` / `in` は**検査しない**（仕様・タスク 4.4 で撤回）──────
            //
            // ⚠⚠ **一度厳密化を入れたが撤回した。** `<` が厳密なのに `==` が無検査という
            //    非対称は**意図された設計**で、`examples/basics/equality_numeric_promotion.ar`
            //    （B2-b）が仕様として固定している:
            //      - 等値は「値の同一性（型厳密）」と「式としての比較（昇格してから比べる）」の
            //        **二層**に分かれている
            //      - 昇格ラティスは `uint → int → float` の一方向で **`bool` は対象外**
            //      - `1 == "a"` / `1 == None` / `[1] == [1.0]` はいずれも **`False`**（エラーではない）
            //    順序比較（`<`）は異型に**意味のある答えが無い**のでエラー、等値は
            //    **常に意味のある答え（偽）がある**ので通す — という区別だった。
            //    ⇒ 検体 `O4` / `O5` / `O8` は「塞ぐべき穴」ではなく**仕様**。検体側に明記した。
            _ => {}
        }
    }

    /// 算術・ビット演算の被演算子が演算可能かを検査する（Kind 3・タスク 4.4）。
    ///
    /// ⚠ **クラスは素通し。** `__add__` 等のダンダーメソッドで演算を定義できるので、
    /// `NamedInstance` / `GenericInstance` が片方にあれば可否を決められない
    /// （`ordered_comparable` が `NamedInstance` を常に許しているのと同じ方針）。
    /// ⚠ `Unresolved` / `Never` も判定材料が無いので素通し（取りこぼす方へ倒す）。
    fn check_arith(&mut self, op: &BinOp, lt: &InferredType, rt: &InferredType, span: Span) {
        use InferredType as T;
        let opaque = |t: &T| {
            matches!(
                t,
                T::Unresolved
                    | T::Never
                    | T::NamedInstance(_)
                    | T::GenericInstance { .. }
                    | T::SelfType
                    | T::Protocol(_)
                    | T::Intersection(_)
                    | T::Namespace(_)
                    | T::PyNamespace(_)
            )
        };
        if opaque(lt) || opaque(rt) {
            return;
        }
        if !matches!(Self::infer_binop_result(op, lt, rt), T::Unresolved) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::IncompatibleBinOp {
                op: op.as_str().to_string(),
                left: lt.clone(),
                right: rt.clone(),
            },
            span: Some(span),
        });
    }

    /// 順序比較演算子 (`<`, `>`, `<=`, `>=`) の型整合性を検査し、不整合があればエラーを記録する。
    fn check_ordered_cmp(
        &mut self,
        lt: &InferredType,
        rt: &InferredType,
        op: &'static str,
        span: Span,
    ) {
        if !Self::ordered_comparable(lt, rt) {
            self.report_error(StaticTypeError::incompatible_cmp(
                lt.clone(),
                rt.clone(),
                op,
                span,
            ));
        }
    }

    /// 2 つの型が順序比較可能な組み合わせかどうかを判定する。
    /// `NamedInstance` は `__lt__` 等のダンダーメソッドを持つ可能性があるため常に許可する。
    fn ordered_comparable(lt: &InferredType, rt: &InferredType) -> bool {
        use InferredType::*;
        if matches!(lt, NamedInstance(_)) || matches!(rt, NamedInstance(_)) {
            return true;
        }
        matches!(
            (lt, rt),
            (Unresolved, _)
                | (_, Unresolved)
                | (Int, Int)
                | (Float, Float)
                | (Int, Float)
                | (Float, Int)
                | (Str, Str)
        )
    }

    /// 2 つの型の join（同じならその型・違えば `Union`）。連結演算の要素型合成に使う。
    /// ⚠ `join_elem_types`（コレクションリテラル用）と同じ規則を 2 項に限った形。
    fn join2(a: &InferredType, b: &InferredType) -> InferredType {
        if a == b {
            a.clone()
        } else {
            InferredType::Union(vec![a.clone(), b.clone()])
        }
    }

    /// 複合代入 `x <op>= v` の**二段検査**（決定 D-8・タスク 5.1）。
    ///
    /// ⚠⚠ **右辺 `v` を左辺型と直接照合してはいけない。** 格納されるのは `v` ではなく
    /// `x <op> v` の**結果**で、照合すると**両方向にずれる**（実測した反例）:
    ///
    /// | コード | 右辺を直接照合すると | 正しい判定 |
    /// |---|---|---|
    /// | `mut t: str = "a"; t *= 3` | ⛔ 誤って弾く（右辺は `int`） | ✅ 通す（`str * int` → `str`） |
    /// | `class Vec: fn __add__(..) -> str`, `v += Vec(2)` | ✅ 通してしまう（右辺は `Vec`） | ⛔ 弾く（結果が `str`） |
    ///
    /// ⇒ **検査 1**（Kind 3: `x <op> v` が可能か）→ 結果型 `R` → **検査 2**（`R` → `typeof(x)`）。
    ///
    /// `target` はエラー文言に出す左辺の綴り（`x` / `c.n`）。
    pub(super) fn check_compound_assign(
        &mut self,
        target: &str,
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
        span: Option<Span>,
    ) {
        // ── 検査 1: 演算そのものの可否（Kind 3）─────────────────────────────
        //
        // ⚠⚠ 以前は複合代入から `check_binop` を**一度も呼んでいなかった**。`x + "s"` は
        //    タスク 4.4 で静的エラーになったのに、`x += "s"` は実行時まで判らなかった
        //    （検体 `B6` / `F2` が `RUNTIME` だった理由）。
        self.check_binop(op, lt, rt, span.clone().unwrap_or_else(Span::unknown));
        // ── 検査 2: 結果型が左辺へ代入できるか（Kind 1）──────────────────────
        let result = self.binop_result_type(op, lt, rt);
        // 判定材料が無い型は取りこぼす方へ倒す（`check_arith` と同じ方針）。
        if matches!(
            result,
            InferredType::Unresolved | InferredType::Never | InferredType::Any
        ) || matches!(
            lt,
            InferredType::Unresolved | InferredType::Never | InferredType::Any
        ) {
            return;
        }
        // ⚠ テンプレートクラスの型変数は具体型と突き合わせられない（`mentions_type_param`）。
        if self.mentions_type_param(lt) || self.mentions_type_param(&result) {
            return;
        }
        // ⚠ 代入先は記憶域なので `Site::Other`（`__cast__` は挿入されない・タスク 4.3）。
        if self.types_compatible(&result, lt, super::type_utils::Site::Other) {
            return;
        }
        self.report_error(StaticTypeError {
            kind: TypeErrorKind::CompoundAssignResultMismatch {
                target: target.to_string(),
                op: op.as_str().to_string(),
                result,
                expected: lt.clone(),
            },
            span,
        });
    }

    /// 二項演算の結果型を、**クラスの演算子メソッドも見て**求める（D-9）。
    ///
    /// ⚠ [`Self::infer_binop_result`] は組み込みの表だけを見るので、クラスが左辺だと
    /// `Unresolved`（＝判定不能）を返す。複合代入の検査 2 はそこで止まってしまうため、
    /// **宣言された戻り値型**を引く経路をここに足した（決定 D-9 の「クラスは `__add__` 等の
    /// 宣言戻り値型を使う」）。
    ///
    /// ⚠⚠ **ダンダー名は実行時の対応表と一致させること**（`interpreter/ops/operators.rs`）。
    /// `Div` は `__truediv__`、`FloorDiv` は `__floordiv__`、ビット演算は
    /// `__and__` / `__or__` / `__xor__` / `__lshift__` / `__rshift__`。
    /// 演算子名から素直に綴った名前（`Div` なら「div」、`BitAnd` なら「bitand」）は
    /// **どれも実在しない**ので、推測で書かずに実行時の表を読むこと。
    /// ずれると静的に見ている演算と実行時に走る演算が食い違う。
    fn binop_result_type(
        &self,
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
    ) -> InferredType {
        let (class_name, subst) = match self.class_and_subst(lt) {
            Some(pair) => pair,
            None => return Self::infer_binop_result(op, lt, rt),
        };
        let method = match op {
            BinOp::Add => "__add__",
            BinOp::Sub => "__sub__",
            BinOp::Mul => "__mul__",
            BinOp::Div => "__truediv__",
            BinOp::FloorDiv => "__floordiv__",
            BinOp::Mod => "__mod__",
            BinOp::Pow => "__pow__",
            BinOp::BitAnd => "__and__",
            BinOp::BitOr => "__or__",
            BinOp::BitXor => "__xor__",
            BinOp::LShift => "__lshift__",
            BinOp::RShift => "__rshift__",
            _ => return InferredType::Unresolved,
        };
        let sigs = match self.registry.class_methods(class_name.as_str()) {
            Some(m) => match m.get(method) {
                Some(sigs) => sigs,
                // ⚠ 演算子メソッドを持たないクラスは**実行時に落ちる**が、ここでは
                //   判定材料が無い扱いにする。可否は検査 1（`check_arith`）の担当で、
                //   そちらもクラスは素通しにしている（`__add__` の有無だけでは
                //   trait 由来の実装や動的な生成を見落とすため）。
                None => return InferredType::Unresolved,
            },
            None => return InferredType::Unresolved,
        };
        // ⚠ オーバーロードがあると実引数で選ぶ必要がある（タスク 5.7）。それまでは
        //   **1 本しか無いときだけ**戻り値型を使う（誤った分岐の戻り値で弾かないため）。
        if sigs.len() != 1 {
            return InferredType::Unresolved;
        }
        match &sigs[0].return_type {
            Some(t) => Self::subst_type_params(t, &subst),
            None => InferredType::Unresolved,
        }
    }

    /// 二項演算子と両辺の型から演算結果の型を推論して返す。
    ///
    /// ⚠⚠ **「結果型」と「可否」を同時に決める**（決定 D-9）。`Unresolved` は
    /// 「この組み合わせは演算できない」を意味し、`check_arith` がそれをエラーにする。
    pub(super) fn infer_binop_result(
        op: &BinOp,
        lt: &InferredType,
        rt: &InferredType,
    ) -> InferredType {
        use InferredType::*;
        if *lt == Any || *rt == Any {
            return Unresolved;
        }
        if matches!(lt, Union(_)) || matches!(rt, Union(_)) {
            return Unresolved;
        }
        // ── set 同士の演算（`|` `&` `^` `-`）───────────────────────────────────
        //
        // ⚠⚠ **素の `Set` だけ見てはいけない。** タスク 2.7 以降 set リテラルは
        //    `SetOf(T)` に推論されるので、`*lt == Set` だけだと `set[int] | set[int]` が
        //    この分岐を外れてビット演算の表へ落ち、**偽エラー**になる
        //    （`collections/collection.ar` が実際に落ちた）。
        // ⚠ 要素型も返す（D-9）。`|` `^` は両方の要素が現れるので join、
        //    `&` `-` は結果が左辺の部分集合なので左辺の要素型。
        // ⚠ 戻り値を明示する。`Some(None)` だけでは内側の型が決まらない。
        //    外側 `Some` = set である／内側 `Some` = 要素型が判っている。
        // ⚠⚠ **`use InferredType::*` が効いているので `None` は `InferredType::None` に
        //    解決される。** `Option` の `None` は `Option::None` と明示すること
        //    （ここで 1 度コンパイルエラーにして気づいた）。
        let set_elem = |t: &InferredType| -> Option<Option<InferredType>> {
            match t {
                Set => Some(Option::None),
                SetOf(e) => Some(Some((**e).clone())),
                _ => Option::None,
            }
        };
        if let (Some(a), Some(b)) = (set_elem(lt), set_elem(rt)) {
            return match op {
                BinOp::BitOr | BinOp::BitXor => match (a, b) {
                    (Some(x), Some(y)) => SetOf(Box::new(Self::join2(&x, &y))),
                    _ => Set,
                },
                BinOp::BitAnd | BinOp::Sub => match a {
                    Some(x) => SetOf(Box::new(x)),
                    Option::None => Set,
                },
                BinOp::Eq | BinOp::NotEq => Bool,
                _ => Unresolved,
            };
        }
        match op {
            BinOp::Eq
            | BinOp::RefEq
            | BinOp::NotEq
            | BinOp::Lt
            | BinOp::Gt
            | BinOp::LtEq
            | BinOp::GtEq
            | BinOp::In
            | BinOp::NotIn => Bool,
            // ── `and` / `or` は**被演算子型の join**（決定 D-10・タスク 2.5）──────
            //
            // ⚠⚠ **以前は無条件 `Bool` と宣言していた。** しかし実行時は
            //    **被演算子をそのまま返す**（Python 流 `a and b` は a か b を返す）ので、
            //    静的には `Bool` と言いながら実行時には非 bool が入る **嘘**になっていた:
            //      let a: int = 1
            //      let b: int = 2
            //      let r: bool = a and b    # 静的に通る
            //      print(r)                 # 2   ← bool 変数に int が入る
            //    `__cast__` に続く 2 件目の「型検査が嘘をつく」箇所だった。
            //
            // ⚠ **実行時の意味論は変えない**（案 (a)）。結果型を正直にするだけ。
            //    `x = a or default` の定番が書けなくなる案 (b) は採らなかった。
            // ⚠⚠ これは **D-12（`if`/`while` の条件を `bool` 厳密に）の前提**。
            //    結果型が `Bool` のままだと、条件を厳密にしても `if a and b:`（非 bool）が
            //    静的に通ってしまう。
            // ⚠ `Any` / `Union` の被演算子はこの関数の冒頭で `Unresolved` に落ちている
            //    （`check_binop` が `OperationOnAny` / `OperationOnUnion` を報告済み）ので、
            //    ここで `Union` が入れ子になることはない。
            BinOp::And | BinOp::Or => {
                if lt == rt {
                    lt.clone()
                } else {
                    Union(vec![lt.clone(), rt.clone()])
                }
            }
            // ── 算術・ビット演算（タスク 4.4 で**実測に合わせて正確にした**）────────
            //
            // ⚠⚠ **ここは「結果型」と同時に「可否」も決める**（決定 D-9）。`Unresolved` を
            //    返した組み合わせが `check_arith` でエラーになるので、**取りこぼしは
            //    偽陽性に直結する**。以前は
            //      - `Add` が list / tuple の連結を知らなかった（`[1] + [2]` が Unresolved）
            //      - `Mul` が `str * int` / `list * int` を知らなかった
            //      - `Mod` が `str % 任意`（書式化）を知らなかった
            //      - `Div` が**無条件 `Float`**（`"a" / 2` を見逃す）
            //      - ビット演算が**無条件 `Int`**（`1.5 & 2` / `"a" & 1` を見逃す）
            //    ⇒ 実行時の挙動を 1 つずつ実測して表を作り直した（下のコメントが根拠）。
            // ⚠ `set` 同士は関数冒頭の専用分岐で先に処理される（`|` `&` `^` `-` `==`）。
            BinOp::Add => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                (Str, Str) => Str,
                // 実測: `[1] + [2]` → `[1, 2]` ／ `(1,) + (2,)` → `(1, 2)`
                // ⚠ 要素型は合成する（連結なのでどちらの要素も現れる）。
                (ListOf(a), ListOf(b)) => ListOf(Box::new(Self::join2(a, b))),
                (List, ListOf(_)) | (ListOf(_), List) | (List, List) => List,
                (Tuple(a), Tuple(b)) => {
                    let mut v = a.clone();
                    v.extend(b.iter().cloned());
                    Tuple(v)
                }
                // ⚠ 実測: `{1} + {2}` は **TypeError**（set の連結は無い）。
                _ => Unresolved,
            },
            BinOp::Sub => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                // 実測: `"ab" - "a"` / `[1] - [2]` は TypeError
                _ => Unresolved,
            },
            BinOp::Mul => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                // 実測: `"ab" * 2` / `2 * "ab"` → "abab" という文字列（`"a" * 1.5` は TypeError）
                (Str, Int) | (Int, Str) => Str,
                // 実測: `[1] * 2` → `[1, 1]` ／ `(1,) * 2` → `(1, 1)`
                (ListOf(a), Int) => ListOf(a.clone()),
                (Int, ListOf(a)) => ListOf(a.clone()),
                (List, Int) | (Int, List) => List,
                (Tuple(a), Int) | (Int, Tuple(a)) => Tuple(a.clone()),
                _ => Unresolved,
            },
            // 実測: `1.5 ** 2` → 2.25 ／ `"ab" ** 2` は TypeError
            BinOp::Pow => match (lt, rt) {
                (Int, Int) => Int,
                (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unresolved,
            },
            // 実測: `1 / 2` → 0.5 ／ **`"a" / 2` は TypeError**（以前は無条件 Float だった）
            BinOp::Div => match (lt, rt) {
                (Complex, Complex)
                | (Complex, Float) | (Float, Complex)
                | (Complex, Int)   | (Int, Complex) => Complex,
                (Int, Int) | (Float, Float) | (Int, Float) | (Float, Int) => Float,
                _ => Unresolved,
            },
            // 実測: `1 // 2` → 0 ／ **`1.5 // 2` は TypeError**（int 同士のみ）
            BinOp::FloorDiv => match (lt, rt) {
                (Int, Int) => Int,
                _ => Unresolved,
            },
            // 実測: `1 % 2` → 1 ／ **`"a" % 2` → `a`・`"a" % 2.5` → `a`**（書式化）
            //       `1.5 % 2` / `2 % "a"` は TypeError
            BinOp::Mod => match (lt, rt) {
                (Int, Int) => Int,
                (Str, _) => Str,
                _ => Unresolved,
            },
            // 実測: `1 & 2` → 0 ／ **`1.5 & 2` / `"a" & 1` は TypeError**
            //       （以前は無条件 `Int` だった）。set 同士は冒頭の専用分岐。
            BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::LShift | BinOp::RShift => {
                match (lt, rt) {
                    (Int, Int) => Int,
                    _ => Unresolved,
                }
            }
        }
    }
}
