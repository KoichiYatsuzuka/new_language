use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Inferred type
// ---------------------------------------------------------------------------

/// 文字列 `s` をブラケット深さ 0 のカンマで分割する。
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// `split_top_level_commas` と同様だが `[` / `]` と `{` / `}` の両方を深さに数える。
fn split_top_level_commas_fn(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' | '{' => depth += 1,
            ']' | '}' => {
                depth = depth.saturating_sub(1);
            }
            ',' if depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// 関数型アノテーションの 1 パラメータ。
///
/// - `name`        : パラメータ名（位置引数の場合は `"param1"` などの自動生成名になる場合もある）
/// - `mutable`     : `mut` 修飾子の有無。`true` なら可変引数として扱う
/// - `ty`          : パラメータの推論済み型
/// - `has_default` : デフォルト値を持つか（＝呼び出しで省略可能か）
#[derive(Debug, Clone, PartialEq)]
pub struct FnTypeParam {
    /// パラメータ名。
    pub name: String,
    /// `mut` 修飾子の有無。
    pub mutable: bool,
    /// パラメータの推論済み型。
    pub ty: InferredType,
    /// デフォルト値を持つか。`true` なら呼び出し時に省略できる。
    ///
    /// ⚠ **型注釈（`fn(int, str)->int`）にはデフォルトを書けない**ので、注釈由来の
    /// `FnTypeParam` は常に `false`。`true` になるのは `Stmt::FnDef` から組み立てる
    /// 経路（`collect_module_types`＝`import` したモジュールのメンバ）だけ。
    pub has_default: bool,
}

/// 型推論システムが扱う型を表す列挙型。プリミティブ型・コレクション型・Union 型・関数型などを網羅する。
#[derive(Debug, Clone, PartialEq)]
pub enum InferredType {
    /// `int` プリミティブ型。
    Int,
    /// `float` プリミティブ型。
    Float,
    /// `complex` プリミティブ型（実部・虚部それぞれ f64）。
    Complex,
    /// `str` プリミティブ型。
    Str,
    /// `bool` プリミティブ型。
    Bool,
    /// `None` 型（値が存在しないことを表す）。
    None,
    /// `Undefined` 型（外部ライブラリのメンバが未定義の状態を表す）。
    /// 変数への代入は禁止。条件判定・型アノテーション・引数としてのみ使用可能。
    Undefined,
    /// 要素型未知のリスト型 `list`。
    List,
    /// 要素型既知のリスト型 `list[T]`。
    ListOf(Box<InferredType>),
    /// 要素型未知の固定長リスト型 `fixed_list`（型検査で変更禁止）。
    FixedList,
    /// 要素型既知の固定長リスト型 `fixed_list[T]`。
    FixedListOf(Box<InferredType>),
    /// `list` と `fixed_list` の両方を受け入れる抽象リスト型 `list_like`。
    ListLike,
    /// 要素型既知の抽象リスト型 `list_like[T]`。
    ListLikeOf(Box<InferredType>),
    /// 型値（クラス自体）を表す `type` 型。型引数なし。
    TypeVal,
    /// 具体的な内部型を持つ型値 `type[T]`（例: `type[int]`, `type[MyClass]`）。
    TypeValOf(Box<InferredType>),
    /// クラス・トレイト本体内でのみ有効な自己参照型 `Self`。
    SelfType,
    /// ユーザー定義クラスのインスタンス型。内部文字列はクラス名。
    NamedInstance(String),
    /// **具体化済みのユーザ定義ジェネリクス**（`Box[int]` / `Pair[int, str]`）。
    ///
    /// ⚠⚠ これが無いと `let x: Box[int]` の型引数が型検査へ届かない。`NamedInstance("Box")`
    /// に落ちると、フィールド型を `class_field_details("Box")` から引いて**置換前の `T`**
    /// を得てしまい、`T` は使用箇所では見えない型変数なので 0-2/0-3 の照合が
    /// `field 'v' of class 'Box' is declared 'T'` という**偽陽性**を出す（実測）。
    ///
    /// ⚠ 等値は**名前と引数の両方**で決まる（`Box[int]` ≠ `Box[str]`）。これは
    /// 実体化ごとに別クラスを作る実行時の扱い（Phase T のメモ化）と整合する。
    GenericInstance {
        /// テンプレートの名前（`Box`）。
        name: String,
        /// 具体化された型引数（宣言順）。
        args: Vec<InferredType>,
    },
    /// プロトコル型。内部文字列はプロトコル名。静的型検査のみで使用。
    /// 変数がこの型の場合、代入時にプロトコル適合チェックが行われる。
    Protocol(String),
    /// 動的型エスケープ `Any`。演算子の型検査を抑制するため明示的なダウンキャストが必要。
    Any,
    /// `Union[T1, T2, ...]` 型。`Option[T]` は `Union[T, None]` の糖衣構文。
    Union(Vec<InferredType>),
    /// `Result[T, E]` 型。成功時の Ok 型 T と失敗時の Err 型 E を保持する特殊な Union 型。
    /// T と E は異なる型でなければならない。ガード節 (`x.is_OK()` / `x.is_ERR()`) なしでは直接使用不可。
    Result(Box<InferredType>, Box<InferredType>),
    /// `Intersection[T1, T2, ...]` 型。すべての構成型のサブクラスであるかプロトコルを満たすことを表す。
    /// 構成型のすべてのメンバーにダウンキャストなしでアクセスできる。
    Intersection(Vec<InferredType>),
    /// 要素型未知の辞書型 `dict`。
    Dict,
    /// キー型・値型既知の辞書型 `dict[K, V]`。
    DictOf(Box<InferredType>, Box<InferredType>),
    /// 要素型未知の集合型 `set`。
    Set,
    /// 要素型既知の集合型 `set[T]`。
    SetOf(Box<InferredType>),
    /// タプル型 `tuple[T1, T2, ...]`。各要素に独立した型を持つ。
    Tuple(Vec<InferredType>),
    /// **要素型も要素数も未知**のタプル型 `tuple`（タスク 8.3）。
    ///
    /// ⚠⚠ `Tuple(Vec<_>)` は**要素数が固定**なので、素の `tuple` 注釈を表せない。
    /// `List` / `Dict` / `Set` と同じ「型引数を書かなかった容器」の位置づけで、
    /// `ListOf` に対する `List` に相当する。名前が `TupleOf` / `Tuple` にならないのは
    /// **既存の `Tuple` が「型引数つき」側**だから（改名は影響が大きいので見送った）。
    ///
    /// ⚠ 適合は **`Tuple(_)` → `TupleAny` の一方向だけ**（D-3）。
    /// 逆を許すと「要素数も型も判らない値」が `tuple[int, str]` として通ってしまう。
    ///
    /// ⚠ 8.1（素の容器型注釈を禁止するか）の判断対象に**これも含まれる**。
    /// `tuple` だけ特別扱いすると `list` / `dict` / `set` と規則が割れるので、
    /// 禁止するなら 4 つまとめて禁止すること。
    TupleAny,
    /// モジュールやパッケージを表す名前空間型。メンバー名 → 推論済み型のマップ。
    Namespace(HashMap<String, InferredType>),
    /// Python モジュール (`import[py]` / `import[py-int]`) を表す名前空間型。
    /// `Namespace` と異なり、未知のメンバーアクセスは `Unresolved` ではなく `Any` を返す。
    PyNamespace(HashMap<String, InferredType>),
    /// 推論に失敗した・または未解決の型（エラーの伝播抑制のため使用する）。
    Unresolved,
    /// **下端型（⊥ / Never）**。決定 D-11・タスク 4.1。
    ///
    /// 「値が 1 つも無い型」で、**あらゆる型へアップキャストできる**。
    /// 空のコレクションリテラル（`[]` / `{}` / `()`）の要素型に使う。
    ///
    /// ⚠⚠ **`Any`（上端）では駄目。方向が逆になる。** `Any` は要素型の上端なので
    /// `list[Any]` は `list[int]` の**スーパータイプ**で、`list[Any]` → `list[int]` は
    /// ダウンキャスト。D-3（アップキャストのみ）のもとでは
    /// `mut xs: list[int] = []` が落ちてしまう（実測）。
    ///
    /// ⚠ **注釈としては書けない**（`from_ann` は解釈しない）。推論の内部表現専用。
    /// ⚠ 注釈なしの束縛（`let xs = []`）は `list[Any]` を既定値にする（D-11 / U-6）。
    /// `Never` のまま束縛すると「要素を足せない空リスト」になってしまう。
    Never,
    /// 関数型 `function[params]->R` または `function{params}->R`。
    /// - `params`: `None` は型引数なし（シグネチャ未確定）、`Some(vec)` は型付きパラメータリスト
    /// - `return_type`: 戻り値の型
    Function {
        /// `None` はシグネチャ未確定（`function` のみ）、`Some(vec)` は型付きパラメータリスト。
        params: Option<Vec<FnTypeParam>>,
        /// 戻り値の型。
        return_type: Box<InferredType>,
    },
}

impl InferredType {
    /// **要素型を知らない容器型**か（タスク 8.1・案 A）。
    ///
    /// ⚠ 注釈位置では弾く（`TypeChecker::check_ann_not_bare`）。
    /// **型テスト位置（`is` / `mustbe` / `case`）では弾かない** —— そこでは
    /// 「list かどうか」を問う正しい用法だから。
    /// ⚠ `Tuple(Vec<_>)`（型引数つき）は**含まない**。含むのは `TupleAny` だけ。
    pub fn is_bare_container(&self) -> bool {
        matches!(
            self,
            Self::List
                | Self::FixedList
                | Self::ListLike
                | Self::Dict
                | Self::Set
                | Self::TupleAny
        )
    }

    /// 型アノテーション文字列を [`InferredType`] に変換する。解析できない場合は `None` を返す。
    pub fn from_ann(ann: &str) -> Option<Self> {
        if let Some(inner) = ann.strip_prefix("Intersection[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            let types: Vec<InferredType> = parts
                .iter()
                .filter_map(|t| InferredType::from_ann(t.trim()))
                .collect();
            return if types.len() >= 2 {
                Some(Self::Intersection(types))
            } else {
                None
            };
        }
        if let Some(inner) = ann.strip_prefix("Result[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            if parts.len() >= 2 {
                if let (Some(ok), Some(err)) = (
                    InferredType::from_ann(parts[0].trim()),
                    InferredType::from_ann(parts[1].trim()),
                ) {
                    return Some(Self::Result(Box::new(ok), Box::new(err)));
                }
            }
            return None;
        }
        if let Some(inner) = ann.strip_prefix("Union[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            let types: Vec<InferredType> = parts
                .iter()
                .filter_map(|t| InferredType::from_ann(t.trim()))
                .collect();
            return if types.len() >= 2 {
                Some(Self::Union(types))
            } else {
                None
            };
        }
        if let Some(inner) = ann
            .strip_prefix("Option[")
            .and_then(|s| s.strip_suffix(']'))
        {
            return InferredType::from_ann(inner.trim()).map(|t| Self::Union(vec![t, Self::None]));
        }
        // ⚠⚠⚠ **以下の容器の「内側が解決できなかったときのフォールバック」を
        //    『到達しない死枝』として消さないこと**（タスク 8.4 の棚卸し結果）。
        //
        //    実測: Arrow のソースからは**どの形でも到達しない**。`parse_type_expr`
        //    （`parser/types.rs`）が先に弾くため:
        //      list[a.b] / set[a.b]        → ParseError: expected `]`, got `.`
        //      dict[str] / list[Result[int]] → ParseError: expected type name, got `]`
        //      Union[int] / Intersection[A]  → ParseError: requires at least 2 type arguments
        //    未知の名前は 8.2 以降 `NamedInstance` になるので、ここへは落ちてこない。
        //
        //    ⚠ **`from_ann` の入力は Arrow のソースだけではない。** `.pyi` スタブ・
        //    `--compile-cs` の `.ars`・py 相互運用の型名変換（`py_type_to_arrow`）は
        //    **パーサを通らない文字列**を渡してくる。スタブによる型検査（タスク 7 の ⑥A）が
        //    入る余地を残すために、この経路は**形を保ったまま残してある**。
        //
        //    ⚠ そのとき「素の容器へ落とす」＝**万能受容体にする**ことの是非は
        //    改めて決めること（8.2 で潰したのと同じ性質の穴になる）。ここを触るなら
        //    実装計画書 §10 の 8.4 と 8.5 を読むこと。
        if let Some(inner) = ann.strip_prefix("list[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::ListOf(Box::new(t)),
                None => Self::List,
            });
        }
        if let Some(inner) = ann.strip_prefix("fixed_list[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::FixedListOf(Box::new(t)),
                None => Self::FixedList,
            });
        }
        if let Some(inner) = ann.strip_prefix("list_like[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::ListLikeOf(Box::new(t)),
                None => Self::ListLike,
            });
        }
        if let Some(inner) = ann.strip_prefix("set[").and_then(|s| s.strip_suffix(']')) {
            return Some(match InferredType::from_ann(inner.trim()) {
                Some(t) => Self::SetOf(Box::new(t)),
                None => Self::Set,
            });
        }
        if let Some(inner) = ann.strip_prefix("dict[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            if parts.len() >= 2 {
                if let (Some(k), Some(v)) = (
                    InferredType::from_ann(parts[0].trim()),
                    InferredType::from_ann(parts[1].trim()),
                ) {
                    return Some(Self::DictOf(Box::new(k), Box::new(v)));
                }
            }
            return Some(Self::Dict);
        }
        if let Some(inner) = ann.strip_prefix("tuple[").and_then(|s| s.strip_suffix(']')) {
            let parts = split_top_level_commas(inner);
            let types: Vec<InferredType> = parts
                .iter()
                .filter_map(|t| InferredType::from_ann(t.trim()))
                .collect();
            return Some(Self::Tuple(types));
        }
        if let Some(inner) = ann.strip_prefix("type[").and_then(|s| s.strip_suffix(']')) {
            let inner = inner.trim();
            let inner_ty = Self::from_ann(inner).or_else(|| {
                if inner
                    .chars()
                    .next()
                    .map(|c| c.is_alphabetic() || c == '_')
                    .unwrap_or(false)
                    && inner.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    Some(Self::NamedInstance(inner.to_string()))
                } else {
                    None
                }
            });
            return inner_ty.map(|t| Self::TypeValOf(Box::new(t)));
        }
        if let Some(rest) = ann.strip_prefix("function") {
            return Self::parse_fn_type_ann(rest);
        }
        // C ABI 型（int32 等）は基底型（int/float）の別名として扱う
        let ann = crate::ast::c_abi_base_type(ann).unwrap_or(ann);
        match ann {
            // ⚠⚠ **`uint` は `int` として解決する**（タスク 6.1）。以前は表に無く
            //    `Unresolved`（＝何でも通る）になっていたので、`let x: uint = "s"` すら
            //    静的には素通りしていた。値としても `Value::Int` が束縛されるので
            //    （`Value::UInt` はハンドル値と `uint()` 変換だけ）、`Int` に寄せるのが
            //    実体と合う。実行時側の相互許容は `primitive_ann_matches` にある。
            "int" | "uint" => Some(Self::Int),
            "float" => Some(Self::Float),
            "complex" => Some(Self::Complex),
            "str" => Some(Self::Str),
            "bool" => Some(Self::Bool),
            "None" => Some(Self::None),
            "Undefined" => Some(Self::Undefined),
            // ⚠⚠⚠ **ここが「素の容器型」を生む唯一の到達可能な地点**（タスク 8.1・案 A）。
            //
            //    ⚠⚠ **純 Arrow コードからは、この結果が注釈として採用されることはない。**
            //    型検査側の注釈入口（`TypeChecker::check_ann_not_bare`）が
            //    `list` / `dict` / `set` / `fixed_list` / `list_like` / `tuple` を
            //    **注釈位置で弾く**ので、`let xs: list = [1]` のような
            //    「要素型を知らない容器」は **Arrow のソースからは作れない**。
            //
            //    ⚠⚠⚠ **それでもこの表を消さないこと。Python 翻訳のために残してある。**
            //    Python の `list` / `dict` / `set` には要素型が無く、翻訳時には
            //    「要素型を知らない容器」を素直に表す型が要る。`from_ann` の入力は
            //    Arrow のソースだけではなく、`.pyi` スタブ・`--compile-cs` の `.ars`・
            //    `py_type_to_arrow` という**パーサを通らない文字列**も来る
            //    （タスク 8.4 の棚卸し ② と同じ経路）。
            //    ⇒ **生成経路は純 Arrow の経路から隔離してある。** Python 翻訳機能を
            //      実装するとき、その経路からここへ繋ぐこと。
            //
            //    ⚠ 型テスト位置（`x is list` / `x mustbe list` / `case list:`）は
            //    **注釈ではない**ので従来どおり通る。そこでの素の容器名は
            //    「list かどうか」を問う正しい用法
            //    （`examples/typing/runtime_type_predicates.ar`）。
            "list" => Some(Self::List),
            "fixed_list" => Some(Self::FixedList),
            "list_like" => Some(Self::ListLike),
            "dict" => Some(Self::Dict),
            "set" => Some(Self::Set),
            // ⚠⚠ **`tuple` は静的側の表に無かった**（タスク 8.3）。`from_ann` が `None` を
            //    返すため注釈が `Unresolved`（＝万能受容体）に化け、
            //      let t: tuple = 1          # 通っていた
            //      let b: tuple = [1, 2]     # list でも通っていた
            //    のように**注釈が完全に無視**されていた（実測）。
            //    ⚠ 8.2 で未知の小文字名を `NamedInstance` 扱いにした結果、この行が無いと
            //      `tuple` が `NamedInstance("tuple")` に化けて**正しい tuple まで弾く**。
            //      ⇒ 8.2 と 8.3 は**同時でなければならない**。
            "tuple" => Some(Self::TupleAny),
            "type" => Some(Self::TypeVal),
            // ⚠⚠ **実行時の型名なのに静的側の表に無かった**（フェーズ 8 で実測）。
            //    `fn counter_from(let n: int) -> generator:` のように**例題が実際に
            //    注釈として使っている**（9 箇所）のに `from_ann` が `None` を返し、
            //    注釈が `Unresolved`（＝万能受容体）に化けていた。
            //    ⚠ `InferredType` に専用の変種は作らず、実行時の型名
            //      （`runtime_type_name` の `Value::Generator(_) => "generator"`）に合わせて
            //      クラス名として扱う。メンバー情報を持たないので存在検査は素通しになる
            //      （タスク 7.5 の `member_set_is_closed`）。
            "generator" => Some(Self::NamedInstance("generator".to_string())),
            "Self" => Some(Self::SelfType),
            "Any" => Some(Self::Any),
            // Unknown identifier that looks like a class name → treat as instance type.
            // This allows `a: Vec2D` parameters to have method calls type-checked correctly.
            //
            // ⚠⚠ **大文字だけでなく小文字始まりの名前もここで受ける**（タスク 8.2）。
            //    以前は小文字の未知名だけが `None`（→ 呼び出し側で `Unresolved` ＝
            //    **万能受容体**）に落ちており、それが
            //      `list[foo]` / `set[foo]` / `dict[str, foo]` / `tuple[int, foo]` /
            //      `Union[int, foo]` / `Intersection[A, foo]`
            //    の **8 箇所すべてで「要素型を黙って捨てる」原因**だった
            //    （`list[foo]` が素の `list` になり、以降の要素検査が全部消える）。
            //    大文字と同じくクラス名扱いにすれば、存在しない型は
            //    **整合性検査が正直な診断で弾く**（`declared 'list[foo]' but … 'list[int]'`）。
            //    ⇒ 脱落地点を 8 か所塗るのではなく、**非対称そのものを消す**。
            // ⚠ `np.ndarray` のような `.` を含む名前は従来どおり `None`（英数と `_` のみ）。
            other if other.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                     && other.chars().all(|c| c.is_alphanumeric() || c == '_') =>
                Some(Self::NamedInstance(other.to_string())),
            // **具体化済みユーザ定義ジェネリクス**（`Box[int]` / `Pair[int,str]`）。
            //
            // ⚠ 組み込みの括弧つき型（`list[...]`・`dict[...]` 等）は上で個別に処理済みなので、
            //   ここへ来るのはユーザ定義テンプレートだけ。`parse_type_expr` が
            //   `known_templates` に載る名前のときだけこの形を作る。
            other if other.ends_with(']') => {
                let open = other.find('[')?;
                let name = &other[..open];
                if name.is_empty()
                    || !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    || !name.chars().all(|c| c.is_alphanumeric() || c == '_')
                {
                    return None;
                }
                let inner = &other[open + 1..other.len() - 1];
                let mut args = Vec::new();
                for part in split_top_level_commas(inner) {
                    // ⚠ 引数が 1 つでも解釈できなければ**型全体を未解決にする**。
                    //   一部だけ解決した型を作ると、置換で嘘の対応付けが生まれる。
                    args.push(Self::from_ann(part.trim())?);
                }
                if args.is_empty() {
                    return None;
                }
                Some(Self::GenericInstance { name: name.to_string(), args })
            }
            _ => None,
        }
    }

    /// `function[...]->R` または `function{...}->R` 形式の関数型アノテーションを解析する。
    fn parse_fn_type_ann(rest: &str) -> Option<Self> {
        let (params, after_params) = if rest.starts_with('[') {
            let close = Self::find_closing_bracket(rest, '[', ']')?;
            let inner = &rest[1..close];
            let after = &rest[close + 1..];
            let params = if inner.trim().is_empty() {
                vec![]
            } else {
                let parts = split_top_level_commas_fn(inner);
                let mut out = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    let p = part.trim();
                    let (mutable, type_str) = if let Some(t) = p.strip_prefix("mut ") {
                        (true, t.trim())
                    } else if let Some(t) = p.strip_prefix("let ") {
                        (false, t.trim())
                    } else {
                        (false, p)
                    };
                    let (name, ty_s) = if let Some(colon) = type_str.find(':') {
                        (
                            type_str[..colon].trim().to_string(),
                            type_str[colon + 1..].trim(),
                        )
                    } else {
                        (format!("param{}", i + 1), type_str)
                    };
                    let ty = Self::from_ann(ty_s).unwrap_or(Self::Any);
                    // 型注釈にデフォルト値は書けないので常に `has_default: false`。
                    out.push(FnTypeParam { name, mutable, ty, has_default: false });
                }
                out
            };
            (Some(params), after)
        } else if rest.starts_with('{') {
            let close = Self::find_closing_bracket(rest, '{', '}')?;
            let inner = &rest[1..close];
            let after = &rest[close + 1..];
            let params = if inner.trim().is_empty() {
                vec![]
            } else {
                let parts = split_top_level_commas_fn(inner);
                let mut out = Vec::new();
                for part in parts.iter() {
                    let p = part.trim();
                    let (mutable, rest_p) = if let Some(t) = p.strip_prefix("mut ") {
                        (true, t.trim())
                    } else if let Some(t) = p.strip_prefix("let ") {
                        (false, t.trim())
                    } else {
                        (false, p)
                    };
                    let colon = rest_p.find(':')?;
                    let name = rest_p[..colon].trim().to_string();
                    let ty_s = rest_p[colon + 1..].trim();
                    let ty = Self::from_ann(ty_s).unwrap_or(Self::Any);
                    // 型注釈にデフォルト値は書けないので常に `has_default: false`。
                    out.push(FnTypeParam { name, mutable, ty, has_default: false });
                }
                out
            };
            (Some(params), after)
        } else {
            (None, rest)
        };

        let return_type = if let Some(ret_s) = after_params.strip_prefix("->") {
            Self::from_ann(ret_s.trim()).unwrap_or(Self::Any)
        } else {
            Self::Any
        };

        Some(Self::Function {
            params,
            return_type: Box::new(return_type),
        })
    }

    /// 対応する閉じブラケットの位置を返す。見つからない場合は `None` を返す。
    fn find_closing_bracket(s: &str, open: char, close: char) -> Option<usize> {
        let mut depth = 0usize;
        for (i, c) in s.char_indices() {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
        }
        None
    }
}

impl std::fmt::Display for InferredType {
    /// [`InferredType`] を人間可読な型アノテーション文字列に変換して表示する。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int => write!(f, "int"),
            Self::Float => write!(f, "float"),
            Self::Complex => write!(f, "complex"),
            Self::Str => write!(f, "str"),
            Self::Bool => write!(f, "bool"),
            Self::None => write!(f, "None"),
            Self::Undefined => write!(f, "Undefined"),
            Self::List => write!(f, "list"),
            Self::ListOf(t) => write!(f, "list[{t}]"),
            // ⚠ 注釈として**読み直せる形**で出すこと（`Box[int]`）。エラーメッセージが
            //   そのまま直し方の提示になる。
            Self::GenericInstance { name, args } => write!(
                f,
                "{name}[{}]",
                args.iter().map(|a| a.to_string()).collect::<Vec<_>>().join(", ")
            ),
            Self::FixedList => write!(f, "fixed_list"),
            Self::FixedListOf(t) => write!(f, "fixed_list[{t}]"),
            Self::ListLike => write!(f, "list_like"),
            Self::ListLikeOf(t) => write!(f, "list_like[{t}]"),
            Self::Dict => write!(f, "dict"),
            Self::DictOf(k, v) => write!(f, "dict[{k},{v}]"),
            Self::Set => write!(f, "set"),
            Self::SetOf(t) => write!(f, "set[{t}]"),
            Self::TypeVal => write!(f, "type"),
            Self::TypeValOf(inner) => write!(f, "type[{inner}]"),
            Self::SelfType => write!(f, "Self"),
            Self::NamedInstance(name) => write!(f, "{name}"),
            Self::Protocol(name) => write!(f, "protocol {name}"),
            Self::Any => write!(f, "Any"),
            Self::Union(types) => {
                if types.len() == 2 && types[1] == Self::None {
                    write!(f, "Option[{}]", types[0])
                } else {
                    let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                    write!(f, "Union[{}]", parts.join(", "))
                }
            }
            Self::Result(ok, err) => write!(f, "Result[{ok}, {err}]"),
            Self::Intersection(types) => {
                let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                write!(f, "Intersection[{}]", parts.join(", "))
            }
            Self::Tuple(types) => {
                let parts: Vec<String> = types.iter().map(|t| t.to_string()).collect();
                write!(f, "tuple[{}]", parts.join(", "))
            }
            Self::TupleAny => write!(f, "tuple"),
            Self::Namespace(members) => write!(f, "<module({} members)>", members.len()),
            Self::PyNamespace(members) => write!(f, "<py-module({} members)>", members.len()),
            Self::Unresolved => write!(f, "unknown"),
            // ⚠ 利用者が書ける綴りではない（推論の内部表現）。表示は空コレクション由来だと
            //    判るものにする。
            Self::Never => write!(f, "never"),
            Self::Function {
                params,
                return_type,
            } => {
                match params {
                    None => write!(f, "function")?,
                    Some(ps) => {
                        let parts: Vec<String> = ps
                            .iter()
                            .map(|p| {
                                let prefix = if p.mutable { "mut" } else { "let" };
                                format!("{prefix} {}:{}", p.name, p.ty)
                            })
                            .collect();
                        write!(f, "function{{{}}}", parts.join(","))?;
                    }
                }
                if **return_type != Self::Any {
                    write!(f, "->{return_type}")?;
                }
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Protocol info
// ---------------------------------------------------------------------------

/// プロトコルのフィールド情報。
#[derive(Debug, Clone)]
pub(crate) struct ProtocolField {
    pub(crate) name: String,
    pub(crate) kind: crate::ast::FieldKind,
    pub(crate) ty: InferredType,
}

/// プロトコルのメソッド情報（シグネチャのみ）。
#[derive(Debug, Clone)]
pub(crate) struct ProtocolMethod {
    pub(crate) name: String,
    /// (param_name, is_mutable, type) ─ self を含まない
    pub(crate) params: Vec<(String, bool, InferredType)>,
    pub(crate) return_type: InferredType,
}

/// プロトコル定義の情報。型検査器が適合チェックに使用する。
#[derive(Debug, Clone)]
pub(crate) struct ProtocolInfo {
    pub(crate) fields: Vec<ProtocolField>,
    pub(crate) methods: Vec<ProtocolMethod>,
}

// ---------------------------------------------------------------------------
// Function signature
// ---------------------------------------------------------------------------

/// 関数シグネチャ情報。パラメータ名・型アノテーション・必須引数数・戻り値型を保持する。
#[derive(Clone, Debug)]
pub(crate) struct FnSig {
    pub(crate) params: Vec<(String, Option<InferredType>)>,
    /// 各パラメータが `mut` か（`params` と同じ並び・同じ長さ）。
    ///
    /// ⚠ 関数の `let` / `mut` は「**関数内で書き換えるか**」の宣言であって、
    /// 変数束縛の規則とは別。`mut` パラメータに `let` 変数を渡すのは
    /// **静的エラー**でなければならない（bug_fix.md B11）。
    pub(crate) param_mutable: Vec<bool>,
    pub(crate) required_count: usize,
    pub(crate) return_type: Option<InferredType>,
    /// 可変長パラメータの要素型。`None` は可変長パラメータなし。
    pub(crate) variadic_type: Option<InferredType>,
}

// ---------------------------------------------------------------------------
// Variable info (scope entry)
// ---------------------------------------------------------------------------

/// スコープ内の変数情報。推論済み型と可変性フラグを保持する。
pub(crate) struct VarInfo {
    pub(crate) ty: InferredType,
    pub(crate) mutable: bool,
}
