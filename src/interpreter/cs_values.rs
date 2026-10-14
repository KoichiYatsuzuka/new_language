// cs_values.rs — C#（cs-proc / cs-dll）と Arrow の値の受け渡しの規則（ffi_value_exchange_plan.md 2-1）。
//
// 2 つのブリッジは運び方（JSON / C ABI）が違うだけで、「宣言された型に対して何を受け取ってよいか」は同じ。
// その規則をここに 1 か所にまとめる:
//   - 戻り値は型スタブの注釈（`str` / `Option[str]` / `Person` …）で読む。クラスはハンドルを `CsObject` に包む
//   - null は `Option[T]`（C# の `string?` などの nullable 注釈）と `None` / `Any` のときだけ `None`。それ以外は TypeError
//   - コレクション（`list` / `tuple` / `dict` / `set`）と `bytes` は cs-proc だけがコピーで運ぶ（5-2・D-5）。cs-dll は
//     まだ運べない（5-5）。デリゲートなど運べない型の値はエラー（黙って `null` / 整数にしない）
//
// ⚠ 以前は戻り値のクラスやコレクションが整数（C# 側のハンドル番号）になり、リストの引数は C# に `null` として
//   届いていた（`.claude/rules/regulations.md` の「掬いきれなかった選択肢はエラーにする」）。
// ⚠ 型スタブの段階で分かるもの（コレクションを受け渡すメソッドなど）は静的エラーにしてある
//   （`parser/cs_assembly/stub_gen.rs` の `cs_unsupported_reason`）。ここは実行時に初めて分かるもの
//   （`object` で宣言された値の中身・C# の約束違反の null など）と、静的な検査を通らない経路の守り。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::value::{ClassValue, CsObjectData, NamespaceData, Value};

thread_local! {
    /// ブリッジ（cs-dll の DLL / cs-proc のホスト）ごとの、型スタブのクラス。戻り値のハンドルを `CsObject` に
    /// 包むときに、宣言された型の名前から引く。⚠ ブリッジの登録表と同じくスレッドローカル（非同期タスクから
    /// 外部関数を呼べないのはフェーズ 6）。
    static CS_CLASSES: RefCell<HashMap<PathBuf, HashMap<String, Rc<ClassValue>>>> =
        RefCell::new(HashMap::new());
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// import した C# のモジュールのクラスを、ブリッジのパスで控える（`import_cs_dll` / `import_cs_proc`）。
pub(crate) fn register_classes(bridge: &Path, ns: &NamespaceData) {
    let classes: HashMap<String, Rc<ClassValue>> = ns
        .members
        .iter()
        .filter_map(|(k, v)| match v {
            Value::Class(c) => Some((k.clone(), c.clone())),
            _ => None,
        })
        .collect();
    CS_CLASSES.with(|m| {
        m.borrow_mut().insert(canon(bridge), classes);
    });
}

fn lookup_class(bridge: &Path, name: &str) -> Option<Rc<ClassValue>> {
    CS_CLASSES.with(|m| m.borrow().get(&canon(bridge)).and_then(|cs| cs.get(name).cloned()))
}

/// 型スタブのメソッドの、引数（`self` を除く）と戻り値の注釈。オーバーロードは先頭（スタブの並び）を使う。
/// 呼び出しの 4 か所（static・インスタンス・プロパティ・cs-dll / cs-proc）が同じ読み方をするための入口。
pub(crate) fn method_types(class: &ClassValue, method: &str) -> (Vec<Option<String>>, Option<String>) {
    match class.methods.get(method).and_then(|ov| ov.first()) {
        Some(f) => (
            f.params.iter().filter(|p| p.name != "self").map(|p| p.type_ann.clone()).collect(),
            f.return_type.clone(),
        ),
        None => (Vec::new(), None),
    }
}

/// 引数の個数で選んだコンストラクタの、引数（`self` を除く）の注釈（5-1）。
/// 型スタブの `__init__` は公開のコンストラクタごとに 1 つある（同じ個数なら先頭・export の生成器と同じ）。
/// その個数のコンストラクタが無ければ TypeError（⚠ 以前はスタブに型が無く、値の種類で並べていた）。
pub(crate) fn ctor_types(class: &ClassValue, argc: usize) -> Result<Vec<Option<String>>, String> {
    let ctors = class.methods.get("__init__").map(|ov| ov.as_slice()).unwrap_or(&[]);
    ctors
        .iter()
        .map(|f| f.params.iter().filter(|p| p.name != "self").map(|p| p.type_ann.clone()).collect::<Vec<_>>())
        .find(|ps| ps.len() == argc)
        .ok_or_else(|| {
            format!(
                "TypeError: C# class '{}' has no public constructor that takes {argc} argument(s)",
                class.name
            )
        })
}

/// 型スタブの注釈が表す C# の値の種類。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CsKind {
    Void,
    Int,
    Float,
    Bool,
    Str,
    /// このアセンブリの公開クラス（値は `CsObject`）
    Class(String),
    /// `object`（中身で決める）
    Any,
    /// `bytes`（C# の `byte[]`・cs-proc だけ・5-2）
    Bytes,
    /// `list[T]`（C# の配列・`List<T>` など・cs-proc だけ・5-2）
    List(Box<CsDecl>),
    /// `tuple[A, B, …]`（C# の `ValueTuple` / `Tuple`・cs-proc だけ・5-2）
    Tuple(Vec<CsDecl>),
    /// `dict[K, V]`（C# の `Dictionary<K, V>` など・cs-proc だけ・5-2）
    Dict(Box<CsDecl>, Box<CsDecl>),
    /// `set[T]`（C# の `HashSet<T>` など・cs-proc だけ・5-2）
    Set(Box<CsDecl>),
}

impl CsKind {
    /// コピーで運ぶ種類（コレクションと `bytes`）。cs-dll はまだ運べない（5-5）。
    pub(crate) fn is_container(&self) -> bool {
        matches!(self, CsKind::Bytes | CsKind::List(_) | CsKind::Tuple(_) | CsKind::Dict(..) | CsKind::Set(_))
    }
}

/// 宣言された型（`Option[T]` / `T | None` なら `optional`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CsDecl {
    pub(crate) kind: CsKind,
    pub(crate) optional: bool,
    /// 注釈そのもの（メッセージ用）
    pub(crate) shown: String,
}

impl CsDecl {
    /// null（`None`）を受け取ってよいか。
    pub(crate) fn accepts_null(&self) -> bool {
        self.optional || matches!(self.kind, CsKind::Void | CsKind::Any)
    }
}

/// 型スタブの注釈を読む。注釈が無ければ `object` と同じ扱い。
/// コレクションは中身の型まで読む（5-2）。デリゲートなど運べない型は TypeError（型スタブの静的エラーを通らない経路の守り）。
pub(crate) fn decl_of(ann: Option<&str>) -> Result<CsDecl, String> {
    let Some(ann) = ann.map(str::trim) else {
        return Ok(CsDecl { kind: CsKind::Any, optional: true, shown: "Any".to_string() });
    };
    parse_decl(ann)
}

fn parse_decl(ann: &str) -> Result<CsDecl, String> {
    let ann = ann.trim();
    let (inner, optional) = if let Some(i) = ann.strip_prefix("Option[").and_then(|s| s.strip_suffix(']')) {
        (i.trim(), true)
    } else if let Some((a, b)) = ann.split_once('|') {
        match (a.trim(), b.trim()) {
            (t, "None") | ("None", t) => (t, true),
            _ => return Err(format!("TypeError: the C# bridge cannot pass a value of the union type `{ann}`")),
        }
    } else {
        (ann, false)
    };
    let kind = match inner {
        "None" | "void" => CsKind::Void,
        "int" => CsKind::Int,
        "float" => CsKind::Float,
        "bool" => CsKind::Bool,
        "str" => CsKind::Str,
        "Any" => CsKind::Any,
        "bytes" => CsKind::Bytes,
        other => match other.split_once('[').and_then(|(head, rest)| Some((head, rest.strip_suffix(']')?))) {
            Some((head, args)) => {
                let args = split_top_level(args);
                let one = |i: usize| parse_decl(args[i]).map(Box::new);
                match (head, args.len()) {
                    ("list", 1) => CsKind::List(one(0)?),
                    ("set", 1) => CsKind::Set(one(0)?),
                    ("dict", 2) => CsKind::Dict(one(0)?, one(1)?),
                    ("tuple", n) if n > 0 => CsKind::Tuple(args.iter().map(|a| parse_decl(a)).collect::<Result<_, _>>()?),
                    _ => return Err(format!("TypeError: a value of type `{other}` cannot cross the C# bridge")),
                }
            }
            None if matches!(other, "list" | "dict" | "set" | "tuple" | "function") => {
                return Err(format!("TypeError: a value of type `{other}` cannot cross the C# bridge"));
            }
            None => CsKind::Class(other.to_string()),
        },
    };
    Ok(CsDecl { kind, optional, shown: ann.to_string() })
}

/// `int, dict[str, int]` を最上位のカンマで分ける。
fn split_top_level(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(s[start..].trim());
    out
}

/// C# から null（cs-proc の `null` / cs-dll の NULL の文字列・ハンドル 0）が返ってきたとき。
/// ⚠ 宣言が null を許さないのに null が来たら TypeError。nullable 注釈の無いアセンブリ（注釈 0）の参照型も
///   `T` として宣言しているので、ここで止まる（Kotlin がプラットフォーム型の値を null にならない型へ入れる
///   ときに検査するのと同じ扱い）。以前は `None` が黙って `str` の値として流れていた。
pub(crate) fn null_result(decl: &CsDecl, what: &str) -> Result<Value, String> {
    if decl.accepts_null() {
        Ok(Value::None)
    } else {
        Err(format!(
            "TypeError: C# method '{what}' returned null, but its return type is `{}`, not `Option[{}]` \
             (declare it nullable in C#, e.g. `string?`)",
            decl.shown, decl.shown
        ))
    }
}

/// C# のオブジェクトのハンドルを、宣言された型で受け取る（クラスなら `CsObject`）。
pub(crate) fn object_result(
    decl: &CsDecl,
    bridge: &Path,
    is_proc: bool,
    handle: i64,
    what: &str,
) -> Result<Value, String> {
    match &decl.kind {
        CsKind::Class(name) => {
            if handle == 0 {
                return null_result(decl, what);
            }
            let class = lookup_class(bridge, name).ok_or_else(|| {
                format!("TypeError: C# method '{what}' returned a `{name}`, but the module has no class `{name}`")
            })?;
            Ok(Value::CsObject(Rc::new(CsObjectData {
                class_name: name.clone(),
                handle,
                bridge_path: bridge.to_path_buf(),
                class,
                is_proc,
            })))
        }
        CsKind::Any => Err(format!(
            "TypeError: C# method '{what}' returned an object whose class Arrow does not know \
             (it is declared `object`; return a class of this assembly instead)"
        )),
        _ => Err(format!(
            "TypeError: C# method '{what}' returned an object, but its return type is `{}`",
            decl.shown
        )),
    }
}

/// 宣言と違う種類の値が返ってきたとき（ブリッジの約束違反）。
pub(crate) fn mismatched_result(decl: &CsDecl, got: &str, what: &str) -> String {
    format!("TypeError: C# method '{what}' returned {got}, but its return type is `{}`", decl.shown)
}

/// 運べない値を引数に渡されたとき。
pub(crate) fn unsupported_arg(v: &Value, index: usize, what: &str) -> String {
    format!(
        "TypeError: argument {} of C# method '{what}' is a '{}', which cannot cross the C# bridge yet",
        index + 1,
        crate::interpreter::ops::typecheck::runtime_type_name(v)
    )
}

/// `CsObject` を別のブリッジ（別の DLL・別のホスト・cs-dll と cs-proc）へ渡したとき。ハンドル番号はブリッジごとの
/// 表の添字なので、そのまま渡すと別のオブジェクトを指す。
pub(crate) fn check_same_bridge(o: &CsObjectData, bridge: &Path, is_proc: bool, index: usize, what: &str) -> Result<(), String> {
    if o.is_proc == is_proc && canon(&o.bridge_path) == canon(bridge) {
        Ok(())
    } else {
        Err(format!(
            "TypeError: argument {} of C# method '{what}' is a C# object from another bridge ('{}')",
            index + 1,
            o.bridge_path.display()
        ))
    }
}

/// 呼び出し元がエラーに付ける接頭辞（`CsProc:` / `CsDll:`）。例外の種類が付いたエラー（`TypeError: …`）には付けない
/// （付けると `except TypeError` で捕まえられなくなる・`make_internal_raised_error`）。
pub(crate) fn tag_error(prefix: &str, e: String) -> String {
    const TYPED: &[&str] = &["TypeError:", "ValueError:", "OverflowError:"];
    if TYPED.iter().any(|t| e.starts_with(t)) {
        e
    } else {
        format!("{prefix}: {e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_declared_types() {
        let d = decl_of(Some("Option[str]")).unwrap();
        assert_eq!((d.kind, d.optional), (CsKind::Str, true));
        let d = decl_of(Some("str | None")).unwrap();
        assert_eq!((d.kind, d.optional), (CsKind::Str, true));
        let d = decl_of(Some("Person")).unwrap();
        assert_eq!((d.kind, d.optional), (CsKind::Class("Person".to_string()), false));
        assert_eq!(decl_of(Some("None")).unwrap().kind, CsKind::Void);
        assert_eq!(decl_of(None).unwrap().kind, CsKind::Any);
        for bad in ["function", "list", "Box[int]", "int | str", "dict[str]"] {
            assert!(decl_of(Some(bad)).unwrap_err().starts_with("TypeError:"), "{bad}");
        }
    }

    /// コレクションは中身の型まで読む（5-2）。
    #[test]
    fn reads_container_declarations() {
        let d = decl_of(Some("dict[str, list[Option[int]]]")).unwrap();
        let CsKind::Dict(k, v) = d.kind else { panic!("dict") };
        assert_eq!(k.kind, CsKind::Str);
        let CsKind::List(e) = v.kind else { panic!("list") };
        assert_eq!((e.kind.clone(), e.optional), (CsKind::Int, true));
        let t = decl_of(Some("Option[tuple[int,str]]")).unwrap();
        assert!(t.optional && matches!(&t.kind, CsKind::Tuple(xs) if xs.len() == 2));
        assert_eq!(decl_of(Some("bytes")).unwrap().kind, CsKind::Bytes);
        assert!(decl_of(Some("set[Person]")).unwrap().kind.is_container());
    }

    #[test]
    fn null_needs_a_nullable_declaration() {
        assert!(matches!(null_result(&decl_of(Some("Option[str]")).unwrap(), "P.m"), Ok(Value::None)));
        assert!(matches!(null_result(&decl_of(Some("None")).unwrap(), "P.m"), Ok(Value::None)));
        assert!(matches!(null_result(&decl_of(Some("Any")).unwrap(), "P.m"), Ok(Value::None)));
        let e = null_result(&decl_of(Some("str")).unwrap(), "P.m").unwrap_err();
        assert!(e.starts_with("TypeError: C# method 'P.m' returned null"), "{e}");
    }

    #[test]
    fn typed_errors_keep_their_type() {
        assert_eq!(tag_error("CsProc", "TypeError: x".to_string()), "TypeError: x");
        assert_eq!(tag_error("CsProc", "pipe closed".to_string()), "CsProc: pipe closed");
    }
}
