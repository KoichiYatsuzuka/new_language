use super::types::InferredType;
use crate::token::Span;

// ---------------------------------------------------------------------------
// Error kind
// ---------------------------------------------------------------------------

/// 静的型エラーの種別を表す列挙型。各バリアントに診断に必要な情報を保持する。
#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// 互換性のない型同士の比較演算（例: `int == str`）。
    IncompatibleComparison {
        lhs: InferredType,
        rhs: InferredType,
        op: &'static str,
    },
    /// 不変変数（`let` / `const`）への代入。
    /// `let` の値に対して**中身を書き換えるメソッド**を呼んだ（bug_fix.md B8）。
    ///
    /// ⚠ 添字代入（`a[0] = 9`）は元から [`TypeErrorKind::AssignToImmutable`] で
    /// 弾いていたのに、**メソッド経由（`a.append(9)`）だけ素通り**していた。
    MutatingMethodOnImmutable {
        /// 呼ばれたメソッド名（`append` など）。
        method: String,
        /// レシーバのパスの**根**になっている変数名（`a.xs[0].append()` なら `a`）。
        root_name: String,
    },
    AssignToImmutable {
        name: String,
    },
    /// 関数呼び出しの引数数が宣言と一致しない（少なすぎる / 多すぎる）。
    CallArgCountMismatch {
        func_name: String,
        /// 最小必須引数数（デフォルト引数を除いた引数数）。
        expected_min: usize,
        /// 最大許容引数数（全引数数）。
        expected_max: usize,
        /// 実際に渡された引数数。
        got: usize,
    },
    /// 関数呼び出しの特定位置の引数型が宣言の型と不一致。
    CallArgTypeMismatch {
        func_name: String,
        /// 0始まりの引数インデックス。
        param_index: usize,
        expected: InferredType,
        got: InferredType,
    },
    /// 関数パラメータに型アノテーションがない。
    MissingParamTypeAnn {
        func_name: String,
        param_name: String,
    },
    /// 関数定義に戻り値型アノテーションがない。
    MissingReturnTypeAnn {
        func_name: String,
    },
    /// 関数に存在しないキーワード引数が渡された。
    UnknownKeywordArg {
        func_name: String,
        arg_name: String,
    },
    /// オーバーロード関数でどのオーバーロードにも引数数が一致しない。
    NoMatchingOverload {
        func_name: String,
        /// 実際に渡された引数数。
        got: usize,
        /// 各オーバーロードが受け付ける引数数のリスト。
        available: Vec<usize>,
    },
    /// メソッドの `self` / `cls` パラメータの型注釈がクラス名と不一致。
    SelfTypeMismatch {
        method: String,
        param_name: String,
        expected_class: String,
        got_class: String,
    },
    /// `Any` 型の値に対して演算子を適用した（明示的ダウンキャストが必要）。
    OperationOnAny {
        op: String,
    },
    /// `Union` 型の値に対して演算子を適用した（明示的ダウンキャストが必要）。
    OperationOnUnion {
        union_type: String,
        op: String,
    },
    /// `is not` 型ガードを非 Union 型に適用した（Union/Optional 型にのみ有効）。
    IsNotOnNonUnion {
        var_name: String,
        var_type: InferredType,
    },
    /// `mut` パラメータに不変変数（`let`/`const`）を渡した。
    CallMutParamWithImmutableArg {
        func_name: String,
        param_name: String,
    },
    /// デコレータの型シグネチャが無効（引数型や戻り値型が不一致など）。
    InvalidDecorator {
        reason: String,
    },
    /// タプルアンパック宣言の変数に `let` / `mut` 修飾子がない。
    TupleUnpackMissingQualifier {
        name: String,
    },
    /// タプルアンパックのターゲット数と右辺タプルの要素数が一致しない。
    TupleUnpackArityMismatch {
        /// 右辺タプルの要素数。
        tuple_len: usize,
        /// タプルアンパックで宣言されたターゲット数（ワイルドカードを除く）。
        target_count: usize,
        /// ワイルドカード `_` が含まれているかどうか。
        has_wildcard: bool,
    },
    /// 不変フィールド（`let` フィールド）への代入（`__init__` 外）。
    AssignToImmutableField {
        field_name: String,
        class_name: String,
    },
    /// `let`/`mut`/`const` の型注釈と初期化子の型が食い違う（0-1）。
    VarTypeMismatch {
        name: String,
        expected: InferredType,
        got: InferredType,
    },
    /// `return` の値の型が宣言された戻り値型と食い違う（0-3）。
    ReturnTypeMismatch {
        func_name: String,
        expected: InferredType,
        got: InferredType,
    },
    /// フィールドの宣言型と代入値の型が食い違う（`o.f = v` / `self.f = v`）。
    ///
    /// ⚠ 以前は**静的にはまったく検査されず**、実行時に捕まるかどうかは
    /// クラスが raw レイアウトを持つか（＝全フィールドが int/float 系プリミティブか）
    /// という**メモリレイアウトの都合**で決まっていた。`str` フィールドが 1 つ混ざるか
    /// trait を 1 つ実装するだけで検査が消えていた。
    FieldTypeMismatch {
        field_name: String,
        class_name: String,
        expected: InferredType,
        got: InferredType,
    },
    /// `private` メンバーにクラス外からアクセスした。
    PrivateAccessError {
        member_name: String,
        class_name: String,
    },
    /// `protected` メンバーに継承クラス外からアクセスした。
    ProtectedAccessError {
        member_name: String,
        class_name: String,
    },
    /// `static` メソッドをインスタンス経由で呼び出した（クラス名経由でのみ呼び出せる）。
    StaticMethodOnInstance {
        method_name: String,
        class_name: String,
    },
    /// `for`/`while` 式の直下に `block_return` が使われた（`loop_yield` か内側のブロック式に移す必要がある）。
    BlockReturnInLoopExpr,
    /// `raise` の対象が例外インスタンスでない。
    InvalidRaiseType {
        got: InferredType,
    },
    /// `mut` / `let` フィールドにデフォルト値が指定された（`const` フィールドのみデフォルト値を持てる）。
    FieldDefaultNotAllowed {
        field_name: String,
        /// フィールドの種別文字列（`"mut"` または `"let"`）。
        kind: String,
    },
    /// `__freeze__` メソッドを直接呼び出した（`freeze` キーワード経由でのみ使用可能）。
    DirectFreezeCall,
    /// プロトコルをインスタンス化しようとした（`MyProtocol()` はエラー）。
    ProtocolInstantiation {
        protocol_name: String,
    },
    /// 型がプロトコルを満たさない（フィールドまたはメソッドが存在しない・型が不一致）。
    ProtocolConformanceFailed {
        type_name: String,
        protocol_name: String,
        reason: String,
    },
    /// 仮引数の**既定値**の型が、その仮引数の宣言型と食い違う（0-12）。
    ///
    /// ⚠⚠ 既定値の式は**推論すらされていなかった**。`check_fn_def` は
    /// 注釈の有無しか見ず `param.default` に触れず、`declare_param` も無視していたため、
    /// `fn f(let n: int = "wrong")` が静的にも実行時にも通っていた（実測）。
    ParamDefaultTypeMismatch {
        func_name: String,
        param_name: String,
        expected: InferredType,
        got: InferredType,
    },
    /// テンプレート（クラス／関数）を**型引数なしで**呼んだ。
    ///
    /// ⚠ Arrow に暗黙実体化は無い（`add_all(3, 4)` は通らない）。実行時は
    /// `TemplateError: template must be called with explicit type arguments` になるが、
    /// 静的エラーが先に出るとそちらに到達しないので、**同じことを静的に言う**。
    /// ⚠ これが無いと、シグネチャの型変数が具体型と突き合わされて
    /// `argument 0 of 'Box.__init__' expects 'T' but got 'int'` という
    /// **型変数を漏らした読めないメッセージ**になっていた（実測）。
    TemplateMissingTypeArgs {
        name: String,
    },
    /// クラス本体に**仮想メソッド**（本体が `...`）を書いた。
    ///
    /// ⚠ 仮想メソッドは **trait 専用の機能**。クラスに置くと「実装を強制する相手」が
    /// 居ないので、以前は**黙って `None` を返す no-op** になっていた（`-> int` と
    /// 宣言しているのに `None` が返る）。
    ///
    /// ⚠⚠ 外部言語のスタブクラス（`import[cs-dll]` 等）は `...` 本体のメソッドを持つが、
    /// あれは「本体が向こう側にある」宣言であって仮想メソッドではない。
    /// ⇒ 検査対象は **Arrow ソースで宣言されたクラス**だけ（`arrow_class_names`・#27-a）。
    VirtualMethodInClass {
        class_name: String,
        method_name: String,
    },
    /// 基底 trait の要求（フィールド型・メソッドシグネチャ）をクラスが満たさない。
    ///
    /// ⚠ protocol（構造的適合）と違い trait は**基底に書く**ので「実装し忘れ」は
    /// パーサが検出する（`must override virtual method`）。ここが見るのは
    /// **同名で宣言はあるが中身が食い違う**場合。
    TraitConformanceFailed {
        class_name: String,
        trait_name: String,
        reason: String,
    },
    /// プロトコルを継承しようとした（`class Foo(MyProtocol):` はエラー）。
    // TODO(reserved): 未発火の診断（Protocol 継承チェック未配線）。実装時に allow を外す。
    #[allow(dead_code)]
    ProtocolInheritance {
        class_name: String,
        protocol_name: String,
    },
    /// `Undefined` リテラルを変数に代入しようとした。
    /// 条件判定・型アノテーション・引数としての使用は許可される。
    AssignUndefined,
    /// `gen` 本体の直下以外で `yield` を書いた（bug_fix.md B13）。
    ///
    /// ⚠⚠ 体裁の問題ではなく、**コルーチン化の前提**。`yield` が自分のフレーム以外に
    /// 現れうると、中断が「`vm::run` を 1 回抜けるだけ」で済まなくなる。
    /// ⚠ 以前はどちらも素通りしていた —— 非 `gen` の `fn` の `yield` は値を捨てて
    ///   `None` を返し、`gen` の中の入れ子 `fn` は呼ばれなければ露見しなかった（実測）。
    YieldOutsideGenerator,
    /// 既にアクセス可能なスコープに同名の変数が存在する状態で再宣言しようとした。
    VariableRedeclaration {
        name: String,
    },
    /// `Result[T, E]` の Ok 型と Err 型が同一または相互に is 判定が成立する。
    ResultSameTypes {
        ok_type: InferredType,
        err_type: InferredType,
    },
    /// 交差型の構成型間で同名のフィールドまたはメソッドが競合している（型・アクセス属性が不一致など）。
    IntersectionMemberConflict {
        member_name: String,
        type_a: String,
        type_b: String,
        reason: String,
    },
    /// 交差型の型ガード節で指定した型が、交差型の構成型制約を満たさない。
    IntersectionGuardTypeFails {
        guard_type: String,
        intersection_type: String,
        reason: String,
    },
    // ── 妥当性検査（タスク 3.4）──────────────────────────────────────────────
    //
    // ⚠ 整合性検査（3 分類）とは**別系統**。「2 つの型が適合するか」ではなく
    //   「名前が在るか・個数が合うか・定義自身が成り立つか」を見る。
    /// 算術・ビット演算の被演算子の型が演算できない組み合わせ（Kind 3・タスク 4.4）。
    ///
    /// ⚠ 以前は順序比較（`<` `>` `<=` `>=`）だけを検査しており、`1 + "s"` のような
    /// **最も頻出の型エラー**が実行時まで判らなかった。
    IncompatibleBinOp {
        op: String,
        left: InferredType,
        right: InferredType,
    },
    /// 複合代入 `x <op>= v` の**演算結果**が左辺の宣言型へ代入できない（D-8・タスク 5.1）。
    ///
    /// ⚠⚠ 右辺 `v` を左辺型と直接照合してはいけない。格納されるのは `v` ではなく
    /// `x <op> v` の**結果**なので、両方向にずれる:
    ///   - `mut s: str = "a"; s += "b"` は右辺も `str` で通るが、
    ///     `mut t: str = "a"; t *= 3` は右辺が `int` でも**正しい**（`str * int` は `str`）
    ///   - 逆に `__add__` が左辺と違う型を返すクラスは、右辺が正しくても**結果が壊れる**
    /// ⇒ 検査 1（Kind 3: `x <op> v` が可能か）→ 結果型 `R` → 検査 2（`R` → `typeof(x)`）。
    CompoundAssignResultMismatch {
        target: String,
        op: String,
        result: InferredType,
        expected: InferredType,
    },
    /// `block_return` / `loop_yield` / `yield` の値が囲み構文の宣言型と合わない
    /// （タスク 5.2）。
    ///
    /// ⚠ `keyword` はどの構文かを文言に出すため（3 つとも同じ検査を通る）。
    BlockExprValueMismatch {
        keyword: String,
        expected: InferredType,
        got: InferredType,
    },
    /// 添字まわりの型不一致（タスク 5.2b）。添字の型・スライス境界・添字代入の値を
    /// **1 つの種類**で表す。
    ///
    /// ⚠ `context` に「どこの型か」を入れる（index of ... ／ begin of slice ／
    /// element of ...）。地点ごとに種類を分けると、文言だけ違う枝が 3 本できて
    /// 片方だけ直す形になる。
    SubscriptTypeMismatch {
        context: String,
        expected: InferredType,
        got: InferredType,
    },
    /// 添字アクセスできない型への `obj[i]`（タスク 5.2b）。
    ///
    /// ⚠ 実測: `set` は `'set' object is not subscriptable` で**必ず**実行時エラーになる。
    /// 型検査側は `SetOf(T)` の添字に `T` を返していたので、**嘘の型**が下流へ流れていた。
    NotSubscriptable { container: InferredType },
    /// `match` の `case` パターンが subject と**決して一致しない**型（タスク 5.4）。
    ///
    /// ⚠⚠ これは「型が違う」ではなく「**腕が永久に死ぬ**」という妥当性の指摘。
    /// `UnknownGuardType` と同じ系統で、`==` の異型比較（`False` を返すのが**仕様**）とは
    /// 別物: 比較は「偽」という意味のある答えを返すが、`case` は**到達不能な分岐**になる。
    ///
    /// ⚠ 判定は `==` と同じ昇格ラティス（`uint → int → float`・`bool` は対象外）に従う。
    /// 実測: `match (a: int): case 1.0:` は**一致する**、`case True:` は一致しない。
    CasePatternNeverMatches {
        subject: InferredType,
        pattern: InferredType,
    },
    /// `if` / `while` の条件が `bool` でない（決定 D-12・タスク 5.5）。
    ///
    /// ⚠⚠ Arrow は**真偽性を使わない**。`if 0:` / `if xs:` / `if some_func:` はすべて
    /// 静的エラーで、`if n != 0:` / `if len(xs) > 0:` / `if some_func():` と書く。
    /// 実利は「**常に真になる条件の書き間違い**」（`()` を忘れた `if some_func:` など）を
    /// 静的に捕まえられること。
    ///
    /// ⚠ 実行時の `is_truthy` は**撤去していない**。`not` / `and` / `or` の評価と、
    /// ネイティブ ABI の関数表 export（C ABI なので削除・改名すると FFI が壊れる）で要る。
    ConditionNotBool { keyword: String, got: InferredType },
    /// オーバーロードのどれも**実引数の型**を受け付けない（タスク 5.7）。
    ///
    /// ⚠⚠ [`TypeErrorKind::NoMatchingOverload`] は**個数**だけを見る。個数が合う候補が
    /// 2 つ以上あると、以前はそこで検査を**丸ごと諦めて**いた（`count_matching.len() != 1`
    /// で `return`）。⇒ 個数さえ合えばどんな型でも通る穴になっていた（検体 `C12`）。
    NoOverloadForArgTypes {
        func_name: String,
        got: Vec<InferredType>,
    },
    /// 型ガード（`is T` / `match ... is T`）の型名が存在しない。
    ///
    /// ⚠⚠ これを検査しないと**腕が永久に死ぬ**うえ、腕の中では対象がその
    /// 存在しないクラスへ絞り込まれるので**メンバーアクセスが全て無検査**になる（検体 X6）。
    UnknownGuardType { type_name: String },
    /// `enum` のバリアント値が `int` でない。
    ///
    /// ⚠ 実行時の `build_enum_classes` も同じ検査をするが、**定義が実行されない経路**
    /// （呼ばれない関数の中の `enum`）では見逃していた（検体 N2）。
    EnumVariantNotInt {
        enum_name: String,
        variant: String,
        got: String,
    },
}

// ---------------------------------------------------------------------------
// StaticTypeWarning
// ---------------------------------------------------------------------------

/// 静的型検査で収集される警告の種別。
#[derive(Debug, Clone)]
pub enum TypeWarningKind {
    /// 関数の戻り値型にプロトコルを使用した（使い勝手が悪いため推奨しない）。
    ProtocolReturnType {
        func_name: String,
        protocol_name: String,
    },
    /// Protocol 型変数を含む関数が部分コンパイル対象になったが、スキップされた。
    // TODO(reserved): 未発火の診断（部分コンパイルスキップ警告未配線）。実装時に allow を外す。
    #[allow(dead_code)]
    ProtocolSkippedCompile {
        func_name: String,
        protocol_name: String,
    },
    /// 交差型の構成型間に同名の同一メンバーが存在する（重複）。
    IntersectionMemberDuplicate {
        member_name: String,
        type_a: String,
        type_b: String,
    },
    /// 交差型を含む関数が部分コンパイル対象になったが、スキップされた。
    IntersectionSkippedCompile {
        func_name: String,
    },
    /// `mustbe コレクション[T]` の要素型は実行時にチェックされない。
    MustBeElemTypeUnchecked {
        guard_type: String,
        outer_type: String,
    },
    /// `mustbe function[...]->R` のシグネチャは実行時にチェックされない。
    MustBeFunctionSignatureUnchecked {
        guard_type: String,
    },
}

/// 静的型検査で収集される警告情報。
#[derive(Debug, Clone)]
pub struct StaticTypeWarning {
    pub kind: TypeWarningKind,
    pub span: Option<Span>,
}

impl StaticTypeWarning {
    pub fn detail_str(&self) -> String {
        match &self.kind {
            TypeWarningKind::ProtocolReturnType { func_name, protocol_name } => format!(
                "function {} returns protocol type {}; consider returning a concrete type instead",
                hl_q(func_name), hl_q(protocol_name)
            ),
            TypeWarningKind::ProtocolSkippedCompile { func_name, protocol_name } => format!(
                "function {} uses protocol type {} and cannot be compiled to native code",
                hl_q(func_name), hl_q(protocol_name)
            ),
            TypeWarningKind::IntersectionMemberDuplicate { member_name, type_a, type_b } => format!(
                "intersection has duplicate member {} defined in both {} and {}; only one will be used",
                hl_q(member_name), hl_q(type_a), hl_q(type_b)
            ),
            TypeWarningKind::IntersectionSkippedCompile { func_name } => format!(
                "function {} uses Intersection type and cannot be compiled to native code",
                hl_q(func_name)
            ),
            TypeWarningKind::MustBeElemTypeUnchecked { guard_type, outer_type } => format!(
                "`mustbe {}` only checks that the value is a `{}` at runtime; element type is not verified",
                hl_q(guard_type), hl_q(outer_type)
            ),
            TypeWarningKind::MustBeFunctionSignatureUnchecked { guard_type } => format!(
                "`mustbe {}` only checks that the value is callable at runtime; signature is not verified",
                hl_q(guard_type)
            ),
        }
    }
}

impl std::fmt::Display for StaticTypeWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const Y: &str = "\x1b[33m";
        const X: &str = "\x1b[0m";
        let loc = match &self.span {
            Some(span) => format!("{span}"),
            None => "\x1b[33m<unknown>\x1b[0m".to_string(),
        };
        write!(f, "{loc}: {Y}Warning{X}: {}", self.detail_str())
    }
}

// ---------------------------------------------------------------------------
// StaticTypeError
// ---------------------------------------------------------------------------

/// 静的型検査で収集されるエラー情報。エラー種別とソース位置を保持する。
#[derive(Debug, Clone)]
pub struct StaticTypeError {
    pub kind: TypeErrorKind,
    pub span: Option<Span>,
}

impl StaticTypeError {
    /// 互換性のない型同士の比較エラーを生成するファクトリ関数。
    pub(super) fn incompatible_cmp(
        lhs: InferredType,
        rhs: InferredType,
        op: &'static str,
        span: Span,
    ) -> Self {
        Self {
            kind: TypeErrorKind::IncompatibleComparison { lhs, rhs, op },
            span: Some(span),
        }
    }

    /// イミュータブル変数への代入エラーを生成するファクトリ関数。
    pub(super) fn assign_immutable(name: &str, span: Span) -> Self {
        Self {
            kind: TypeErrorKind::AssignToImmutable {
                name: name.to_string(),
            },
            span: Some(span),
        }
    }

    /// エラーが発生したファイル名を返す。スパン情報がない場合は `"<unknown>"` を返す。
    pub fn file_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 && !span.file.is_empty() => span.file.to_string(),
            _ => "<unknown>".to_string(),
        }
    }

    /// エラー発生位置の行番号と列番号を `"行:列"` 形式で返す。不明な場合は `"-"` を返す。
    pub fn line_col_str(&self) -> String {
        match &self.span {
            Some(span) if span.line != 0 => format!("{}:{}", span.line, span.col),
            _ => "-".to_string(),
        }
    }

    /// エラー種別の固定文字列 `"StaticTypeError"` を返す。
    pub fn error_type_str(&self) -> &'static str {
        "StaticTypeError"
    }

    /// エラーの詳細メッセージ文字列を ANSI エスケープシーケンス付きで返す。
    pub fn detail_str(&self) -> String {
        match &self.kind {
            TypeErrorKind::IncompatibleComparison { lhs, rhs, op } => format!(
                "cannot compare {} and {} with {}", hl_q(lhs), hl_q(rhs), hl_bt(op)
            ),
            TypeErrorKind::MutatingMethodOnImmutable { method, root_name } => format!(
                "cannot call {} on {} — it is immutable",
                hl_q(method), hl_q(root_name)
            ),
            TypeErrorKind::AssignToImmutable { name } => format!(
                "cannot assign to immutable variable {}", hl_q(name)
            ),
            TypeErrorKind::CallArgCountMismatch { func_name, expected_min, expected_max, got } => {
                if expected_min == expected_max {
                    format!("{} takes {expected_min} argument(s) but {got} were given", hl_q(func_name))
                } else {
                    format!("{} takes {expected_min} to {expected_max} argument(s) but {got} were given", hl_q(func_name))
                }
            }
            TypeErrorKind::CallArgTypeMismatch { func_name, param_index, expected, got } => {
                // ⚠ `param_index == usize::MAX` は**可変長引数**を表す番兵（`check_call_args`）。
                //    そのまま出すと `argument 18446744073709551615 of ...` になる
                //    （タスク 5.2c で可変長の検査が効くようになって初めて表に出た）。
                let which = if *param_index == usize::MAX {
                    "the variadic argument".to_string()
                } else {
                    format!("argument {param_index}")
                };
                format!(
                    "{which} of {} expects {} but got {}",
                    hl_q(func_name), hl_q(expected), hl_q(got)
                )
            }
            TypeErrorKind::MissingParamTypeAnn { func_name, param_name } => format!(
                "parameter {} of function {} is missing a type annotation",
                hl_q(param_name), hl_q(func_name)
            ),
            TypeErrorKind::MissingReturnTypeAnn { func_name } => format!(
                "function {} is missing a return type annotation", hl_q(func_name)
            ),
            TypeErrorKind::UnknownKeywordArg { func_name, arg_name } => format!(
                "{} has no parameter named {}", hl_q(func_name), hl_q(arg_name)
            ),
            TypeErrorKind::NoMatchingOverload { func_name, got, available } => {
                let avail = available.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(", ");
                format!("no overload of {} takes {got} argument(s) (overloads take: {avail})", hl_q(func_name))
            }
            TypeErrorKind::SelfTypeMismatch { method, param_name, expected_class, got_class } => format!(
                "parameter {} of {} expects {} = {} but got {}",
                hl_q(param_name), hl_q(method), hl_q("Self"), hl_q(expected_class), hl_q(got_class)
            ),
            TypeErrorKind::OperationOnAny { op } => format!(
                "cannot apply {} to {} — explicit downcast required", hl_bt(op), hl_q("Any")
            ),
            TypeErrorKind::OperationOnUnion { union_type, op } => format!(
                "cannot apply {} to {} — explicit downcast required", hl_bt(op), hl_q(union_type)
            ),
            TypeErrorKind::IsNotOnNonUnion { var_name, var_type } => format!(
                "{} type guard on {} requires a Union or Optional type, but got {}",
                hl_q("is not"), hl_q(var_name), hl_q(var_type)
            ),
            TypeErrorKind::CallMutParamWithImmutableArg { func_name, param_name } => format!(
                "parameter {} of {} expects a mutable argument, but got an immutable value",
                hl_q(param_name), hl_q(func_name)
            ),
            TypeErrorKind::InvalidDecorator { reason } => format!(
                "invalid decorator: \x1b[1;35m{reason}\x1b[0m"
            ),
            TypeErrorKind::TupleUnpackMissingQualifier { name } => format!(
                "variable {} in tuple unpack requires {} or {} qualifier",
                hl_q(name), hl_bt("let"), hl_bt("mut")
            ),
            TypeErrorKind::TupleUnpackArityMismatch { tuple_len, target_count, has_wildcard } => {
                if *has_wildcard {
                    format!("tuple unpack has \x1b[1;35m{target_count}\x1b[0m variable(s) but tuple has only \x1b[1;35m{tuple_len}\x1b[0m element(s)")
                } else {
                    format!("tuple unpack expects \x1b[1;35m{target_count}\x1b[0m element(s) but tuple has \x1b[1;35m{tuple_len}\x1b[0m")
                }
            }
            TypeErrorKind::AssignToImmutableField { field_name, class_name } => format!(
                "cannot assign to immutable field {} of class {}", hl_q(field_name), hl_q(class_name)
            ),
            TypeErrorKind::VarTypeMismatch { name, expected, got } => format!(
                "{} is declared {} but initialized with {}",
                hl_q(name),
                hl_q(&expected.to_string()),
                hl_q(&got.to_string())
            ),
            TypeErrorKind::ReturnTypeMismatch { func_name, expected, got } => format!(
                "{} is declared to return {} but returns {}",
                hl_q(func_name),
                hl_q(&expected.to_string()),
                hl_q(&got.to_string())
            ),
            TypeErrorKind::FieldTypeMismatch { field_name, class_name, expected, got } => format!(
                "field {} of class {} is declared {} but got {}",
                hl_q(field_name),
                hl_q(class_name),
                hl_q(&expected.to_string()),
                hl_q(&got.to_string())
            ),
            TypeErrorKind::PrivateAccessError { member_name, class_name } => format!(
                "{} is private and cannot be accessed outside {}", hl_q(member_name), hl_q(class_name)
            ),
            TypeErrorKind::ProtectedAccessError { member_name, class_name } => format!(
                "{} is protected and cannot be accessed outside {} or its subclasses",
                hl_q(member_name), hl_q(class_name)
            ),
            TypeErrorKind::StaticMethodOnInstance { method_name, class_name } => format!(
                "static method {} must be called on {}, not an instance",
                hl_q(method_name), hl_bt(class_name)
            ),
            TypeErrorKind::BlockReturnInLoopExpr => format!(
                "{} cannot be used directly in a {} or {} expression body; use {} to accumulate values or nest inside an {} / {} / {} expression",
                hl_bt("block_return"), hl_bt("for"), hl_bt("while"),
                hl_bt("loop_yield"), hl_bt("if"), hl_bt("match"), hl_bt("block:")
            ),
            TypeErrorKind::InvalidRaiseType { got } => format!(
                "{} expects an instance implementing trait {}, but got {}",
                hl_bt("raise"), hl_q("Error"), hl_q(got)
            ),
            TypeErrorKind::FieldDefaultNotAllowed { field_name, kind } => format!(
                "{} field {} cannot have a default value in the class declaration; only {} or {} fields may have defaults",
                hl_bt(kind), hl_q(field_name), hl_bt("const"), hl_bt("static mut")
            ),
            TypeErrorKind::DirectFreezeCall => format!(
                "{} cannot be called directly; use the {} keyword instead",
                hl_bt("__freeze__"), hl_bt("freeze")
            ),
            TypeErrorKind::ProtocolInstantiation { protocol_name } => format!(
                "cannot instantiate protocol {}; protocols are for type-checking only",
                hl_q(protocol_name)
            ),
            TypeErrorKind::ProtocolConformanceFailed { type_name, protocol_name, reason } => format!(
                "type {} does not satisfy protocol {}: {}",
                hl_q(type_name), hl_q(protocol_name), reason
            ),
            TypeErrorKind::ParamDefaultTypeMismatch { func_name, param_name, expected, got } => format!(
                "default value of parameter {} of {} is declared {} but got {}",
                hl_q(param_name),
                hl_q(func_name),
                hl_q(&expected.to_string()),
                hl_q(&got.to_string())
            ),
            TypeErrorKind::TemplateMissingTypeArgs { name } => format!(
                "template {} must be called with explicit type arguments (e.g. {})",
                hl_q(name),
                hl_q(&format!("{name}[T](...)"))
            ),
            TypeErrorKind::VirtualMethodInClass { class_name, method_name } => format!(
                "class {} cannot declare virtual method {} (a `...` body);                  virtual methods belong to traits — give it a real body,                  or move the declaration into a trait that {} implements",
                hl_q(class_name), hl_q(method_name), hl_q(class_name)
            ),
            TypeErrorKind::TraitConformanceFailed { class_name, trait_name, reason } => format!(
                "class {} does not satisfy trait {}: {}",
                hl_q(class_name), hl_q(trait_name), reason
            ),
            TypeErrorKind::ProtocolInheritance { class_name, protocol_name } => format!(
                "class {} cannot inherit from protocol {}; use protocol type annotations instead",
                hl_q(class_name), hl_q(protocol_name)
            ),
            TypeErrorKind::YieldOutsideGenerator => format!(
                "{} is only allowed directly inside a {} function",
                hl_bt("yield"), hl_bt("gen")
            ),
            TypeErrorKind::AssignUndefined => format!(
                "cannot assign {} to a variable; {} can only be used in conditions and type annotations",
                hl_bt("Undefined"), hl_bt("Undefined")
            ),
            TypeErrorKind::VariableRedeclaration { name } => format!(
                "variable {} is already declared in an accessible scope",
                hl_q(name)
            ),
            TypeErrorKind::ResultSameTypes { ok_type, err_type } => format!(
                "Result[{}, {}]: Ok type and Err type must be different",
                hl_q(ok_type), hl_q(err_type)
            ),
            TypeErrorKind::IntersectionMemberConflict { member_name, type_a, type_b, reason } => format!(
                "intersection member {} from {} and {} conflict: {}",
                hl_q(member_name), hl_q(type_a), hl_q(type_b), reason
            ),
            TypeErrorKind::IntersectionGuardTypeFails { guard_type, intersection_type, reason } => format!(
                "type {} used in type guard does not satisfy {}: {}",
                hl_q(guard_type), hl_q(intersection_type), reason
            ),
            TypeErrorKind::IncompatibleBinOp { op, left, right } => format!(
                "unsupported operand types for {}: {} and {}",
                hl_q(op), hl_q(&left.to_string()), hl_q(&right.to_string())
            ),
            TypeErrorKind::CompoundAssignResultMismatch { target, op, result, expected } => format!(
                "{} on {} produces {} but {} is declared {}",
                hl_q(op), hl_q(target), hl_q(&result.to_string()),
                hl_q(target), hl_q(&expected.to_string())
            ),
            TypeErrorKind::BlockExprValueMismatch { keyword, expected, got } => format!(
                "{} expects {} but got {}",
                hl_q(keyword), hl_q(&expected.to_string()), hl_q(&got.to_string())
            ),
            TypeErrorKind::SubscriptTypeMismatch { context, expected, got } => format!(
                "{} expects {} but got {}",
                context, hl_q(&expected.to_string()), hl_q(&got.to_string())
            ),
            TypeErrorKind::NotSubscriptable { container } => format!(
                "{} is not subscriptable",
                hl_q(&container.to_string())
            ),
            TypeErrorKind::CasePatternNeverMatches { subject, pattern } => format!(
                "case pattern of type {} can never match a subject of type {}; the branch is dead",
                hl_q(&pattern.to_string()), hl_q(&subject.to_string())
            ),
            TypeErrorKind::ConditionNotBool { keyword, got } => format!(
                "the condition of {} must be {}, got {}",
                hl_q(keyword), hl_q("bool"), hl_q(&got.to_string())
            ),
            TypeErrorKind::NoOverloadForArgTypes { func_name, got } => {
                let list = got.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
                format!(
                    "no overload of {} accepts argument types ({})",
                    hl_q(func_name), hl_q(&list)
                )
            }
            // ── 妥当性検査（タスク 3.4）────────────────────────────────────────
            TypeErrorKind::UnknownGuardType { type_name } => format!(
                "unknown type {} in type guard; the branch can never match",
                hl_q(type_name)
            ),
            TypeErrorKind::EnumVariantNotInt { enum_name, variant, got } => format!(
                "enum variant {} of {} must be int, got {}",
                hl_q(variant), hl_q(enum_name), hl_q(got)
            ),
        }
    }
}

/// 値をシングルクォートで囲み、マゼンタ太字の ANSI 装飾を付けた文字列を返す。
fn hl_q(s: impl std::fmt::Display) -> String {
    format!("'\x1b[1;35m{s}\x1b[0m'")
}

/// 値をバッククォートで囲み、マゼンタ太字の ANSI 装飾を付けた文字列を返す。
fn hl_bt(s: impl std::fmt::Display) -> String {
    format!("`\x1b[1;35m{s}\x1b[0m`")
}

impl std::fmt::Display for StaticTypeError {
    /// エラーを `"位置: StaticTypeError: 詳細"` 形式の ANSI 色付き文字列として出力する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const R: &str = "\x1b[31m";
        const X: &str = "\x1b[0m";
        let loc = match &self.span {
            Some(span) => format!("{span}"),
            None => "\x1b[33m<unknown>\x1b[0m".to_string(),
        };
        write!(f, "{loc}: {R}StaticTypeError{X}: {}", self.detail_str())
    }
}
