// value/core.rs — Value 列挙型(全実行時値のユニオン)・CsObjectData・deep_clone ヘルパー・impl Value・ExecResult(制御フロー信号)・クラスID採番。

use {
    std::cell::RefCell, std::collections::HashMap, std::rc::Rc, std::sync::atomic::{AtomicU32, Ordering}, std::sync::Arc,
    crate::interpreter::async_mgr,
};
use super::*;


// ---------------------------------------------------------------------------
// Class ID registry
// ---------------------------------------------------------------------------

/// 次に割り当てるクラス ID（グローバルアトミックカウンタ）。
static NEXT_CLASS_ID: AtomicU32 = AtomicU32::new(1); // 0 = 未割り当て

/// 新しい一意なクラス ID を発行する。クラス定義時に一度だけ呼ぶ。
#[inline]
pub fn alloc_class_id() -> u32 {
    NEXT_CLASS_ID.fetch_add(1, Ordering::Relaxed)
}


// ---------------------------------------------------------------------------
// Value enum
// ---------------------------------------------------------------------------

/// インタープリタが扱う実行時値の列挙型。
///
/// 各バリアントの概要:
/// - `Int` / `Float` / `Str` / `Bool` / `None`: プリミティブ値
/// - `List`: 可変長リスト（各要素は任意の型）
/// - `Function`: 通常の関数値
/// - `OverloadedFn`: 同名関数のオーバーロード候補リスト（同スコープに複数定義されたとき）
/// - `Class`: クラス定義（コンストラクタとして呼び出し可能）
/// - `Instance`: クラスインスタンス（共有参照かつ内部可変）
/// - `Type`: 組み込み型名（`int`, `str`, `float`, `bool`）。ユーザー定義型は `Class` で表現
/// - `Trait`: 宣言されたトレイトの実行時表現
/// - `TemplateFn` / `TemplateClass` / `TemplateGenFn`: 未実体化のテンプレート関数・クラス・ジェネレータ
/// - `GeneratorFn`: ジェネレータ関数（呼び出すと `Generator` を返す）
/// - `Generator`: 実体化済みジェネレータ（yield 済み値を保持）
/// - `Dict`: 型付き or 型なし辞書（キー・値の並列 Vec で内部管理）
/// - `Tuple`: 不変・固定長のシーケンス（各要素の型情報を保持）
/// - `NativeFunction`: natively compiled function loaded from a shared library
#[derive(Debug, Clone)]
pub enum Value {
    /// 符号付き 64 ビット整数プリミティブ値。
    Int(i64),
    /// 符号なし 64 ビット整数値（ビット演算・ネイティブ ABI との連携用）。
    UInt(u64),
    /// 64 ビット浮動小数点プリミティブ値。
    Float(f64),
    /// 複素数プリミティブ値（実部・虚部はそれぞれ f64）。
    Complex(f64, f64),
    /// 文字列プリミティブ値（Unicode UTF-8）。
    ///
    /// `Rc<str>` なので `Value::clone`（変数読み・引数束縛・スタック push）は
    /// **参照カウント加算のみ**でヒープ確保しない（#15 / §7.4-1）。
    /// 文字列は不変なので共有しても意味論は変わらない。
    /// ただし **`deep_clone` は必ず独立バッファを作ること**（async のスレッド間 share-nothing。
    /// `Rc` の参照カウントは非アトミックなので共有したまま送ると壊れる）。
    Str(Rc<str>),
    /// 真偽値プリミティブ値（`true` / `false`）。
    Bool(bool),
    /// `None` リテラル。Python の `None` に相当する。
    None,
    /// `Undefined` リテラル。外部ライブラリのメンバが未定義の状態を表す特殊値。
    /// 変数への代入は静的型エラー。条件判定（`x is Undefined`）と引数としてのみ使用可能。
    Undefined,
    /// 可変長リスト値。要素は任意の型を混在できる。`Rc<RefCell<...>>` により共有・可変参照する。
    List(Rc<RefCell<Vec<Value>>>),
    /// フラット固定長リスト。全要素が同一クラスかつ全フィールドが int/float。
    /// `state` は可変メタデータ（data/len/allocated_size）、`layout` はレイアウト記述（不変）。
    /// `mut` 変数として宣言すれば `append` で要素を追加でき、`freeze` で余剰確保を解放する。
    FrozenList { state: Rc<RefCell<FlatListData>>, layout: Rc<FlatLayout> },
    /// 通常の関数値（`fn` キーワードで定義された関数）。
    Function(Rc<FnValue>),
    /// 同スコープに同名で2つ以上のオーバーロードが定義された関数値。
    OverloadedFn(Vec<Rc<FnValue>>),
    /// クラス定義値。コンストラクタとして呼び出すとインスタンスを生成する。
    Class(Rc<ClassValue>),
    /// クラスインスタンス値。`Rc<RefCell<...>>` により共有・可変参照する。
    Instance(Rc<RefCell<InstanceData>>),
    /// 組み込み型名を保持する型値（`int`, `str`, `float`, `bool`）。
    /// ユーザー定義クラス型は `Value::Class` で表現する。
    Type(String),
    /// 宣言された trait の実行時表現。
    Trait(String),
    /// 宣言された protocol の実行時表現（型チェック専用; インスタンス化は不可）。
    Protocol(String),
    /// 型変数でパラメータ化された未実体化のテンプレート関数。
    TemplateFn(Rc<TemplateFnValue>),
    /// 型変数でパラメータ化された未実体化のテンプレートクラス。
    TemplateClass(Rc<TemplateClassValue>),
    /// ジェネレータ関数（呼び出すと `Generator` を生成して返す）。
    GeneratorFn(Rc<GeneratorFnValue>),
    /// 型変数でパラメータ化された未実体化のテンプレートジェネレータ関数。
    TemplateGenFn(Rc<TemplateGenFnValue>),
    /// 実体化済みジェネレータ。すべての yield 値を一括収集して保持する。
    Generator(Rc<RefCell<GeneratorState>>),
    /// 型付きまたは型なし（Any）のキー・値を持つ辞書値。
    Dict(Rc<RefCell<DictData>>),
    /// 不変・固定長のシーケンス。各要素に型情報を保持する。
    Tuple(Rc<TupleData>),
    /// 重複なしの可変コレクション。要素の順序は保証しない。
    Set(Rc<RefCell<Vec<Value>>>),
    /// import されたモジュールまたは名前空間。`.` でメンバにアクセスする。
    Namespace(Rc<NamespaceData>),
    /// PyO3 経由で保持する Python オブジェクトへの参照。
    /// tl 側では不透明（opaque）な値として扱われる。
    PyObject(Arc<PyObjHandle>),
    /// ファイルオブジェクト。`open()` 組み込み関数が返す。
    /// メソッド（read / read_line / read_letter / write / write_line）で読み書きを行う。
    FileObject(Rc<RefCell<FileData>>),
    /// Natively compiled function from a shared library.
    /// Call it via `libloading` using the `{fn_name}_tl` exported symbol.
    NativeFunction(Arc<NativeFnRef>),
    /// スライス値: `obj[begin:end:step]` または `slice(begin, end, step)` で生成される。
    /// begin/end は Optional[Index]、step は Optional[int]。
    Slice(Rc<SliceValue>),
    /// AsyncManager インスタンス。`AsyncManager(num_thread=N)` で生成される。
    AsyncManager(Rc<RefCell<async_mgr::AsyncManagerData>>),
    /// Async 列挙型の値 (Async.Waiting / Async.Running / Async.Done)。
    AsyncStatusVal(async_mgr::AsyncStatus),
    /// `Signal[T]()` で生成される Arrow ネイティブのイベントソース。
    /// `.emit(val)` で全ハンドラを同期呼び出し、`.emit_async(val)` でキューに積む。
    Signal(Rc<RefCell<crate::interpreter::event_loop::SignalData>>),
    /// EventLoop シングルトン。`EventLoop.run()` / `EventLoop.post(fn)` で利用する。
    EventLoop(Rc<RefCell<crate::interpreter::event_loop::EventLoopData>>),
    /// import[cs-dll] で生成される C# オブジェクトのハンドル。
    CsObject(Rc<CsObjectData>),
    /// import[js-proc] で生成される JavaScript モジュール関数のランタイム表現。
    /// 呼び出し時に js_proc_runtime 経由で Node.js ブリッジに IPC 送信する。
    /// フィールドは `Box` 化して `size_of::<Value>()` を縮める（§7.4）。稀な値なので
    /// clone の深いコピー（Box::clone = 中身複製）コストは支配的でない。
    JsProcFn(Box<JsProcData>),
    /// `Result[T, E]` 値。`Ok(value)` または `Err(error)` で生成される。
    /// `ok: true` → Ok 側の値、`ok: false` → Err 側の値。
    ResultVal { ok: bool, inner: Box<Value> },
}


/// `Value::JsProcFn` の中身（`Box` 化して `Value` サイズを縮小; §7.4）。
#[derive(Debug, Clone)]
pub struct JsProcData {
    /// ブリッジスクリプトのパス（ブリッジレジストリのキー）。
    pub bridge_key: String,
    /// JS モジュール名（スラッシュ区切り、ブリッジに渡す）。
    pub module_name: String,
    /// 呼び出す関数名。
    pub fn_name: String,
}

/// import[cs-dll] / import[cs-proc] ブリッジが管理する C# オブジェクトのランタイム表現。
#[derive(Debug, Clone)]
pub struct CsObjectData {
    pub class_name: String,
    pub handle: i64,
    /// NativeAOT DLL パス (cs-dll) または proc exe パス (cs-proc)。
    pub bridge_path: std::path::PathBuf,
    /// 元の ClassValue stub（return type 解決に使用）。
    pub class: Rc<ClassValue>,
    /// true = cs-proc (IPC サブプロセス), false = cs-dll (DLL 直接呼び出し)
    pub is_proc: bool,
}


// ---------------------------------------------------------------------------
// deep_clone helpers (used when capturing the environment for async tasks)
// ---------------------------------------------------------------------------

/// Deep-clone a CapturedVar environment map so no Rc is shared across threads.
pub fn deep_clone_captured_env(
    env: &HashMap<String, CapturedVar>,
) -> HashMap<String, CapturedVar> {
    env.iter()
        .map(|(k, v)| {
            let cloned = match v {
                CapturedVar::Immutable(val) => CapturedVar::Immutable(val.deep_clone()),
                CapturedVar::Mutable(cell) => {
                    CapturedVar::Mutable(Rc::new(RefCell::new(cell.borrow().deep_clone())))
                }
            };
            (k.clone(), cloned)
        })
        .collect()
}


impl Value {
    /// 文字列値を作る。`&str` / `String` / `Rc<str>` のいずれからでも書ける（#15）。
    ///
    /// `Rc<str>` を渡した場合は参照カウント加算のみで**確保しない**ので、
    /// 既にある文字列値を使い回す経路（属性名・辞書キー・リテラル）はこれを通すこと。
    #[inline]
    pub fn str(s: impl Into<Rc<str>>) -> Value {
        Value::Str(s.into())
    }

    /// Create a fully independent deep copy with no shared Rc pointers.
    /// Used before sending values across thread boundaries for async tasks.
    pub fn deep_clone(&self) -> Value {
        match self {
            Value::Int(n) => Value::Int(*n),
            Value::UInt(n) => Value::UInt(*n),
            Value::Float(f) => Value::Float(*f),
            Value::Complex(re, im) => Value::Complex(*re, *im),
            // Rc の参照カウントは非アトミック。スレッド間送出では共有せず必ず複製する（#15）。
            Value::Str(s) => Value::Str(Rc::from(&**s)),
            Value::Bool(b) => Value::Bool(*b),
            Value::None => Value::None,
            Value::Undefined => Value::Undefined,
            Value::List(rc) => {
                let v = rc.borrow().iter().map(|x| x.deep_clone()).collect();
                Value::List(Rc::new(RefCell::new(v)))
            }
            Value::FrozenList { state, layout } => {
                let old = state.borrow();
                Value::FrozenList {
                    state: Rc::new(RefCell::new(FlatListData {
                        data: old.data.clone(),
                        len: old.len,
                        allocated_size: old.allocated_size,
                    })),
                    layout: Rc::new(FlatLayout {
                        class_name: layout.class_name.clone(),
                        fields: layout.fields.clone(),
                        stride: layout.stride,
                        class: Rc::new(layout.class.deep_clone()),
                    }),
                }
            }
            Value::Set(rc) => {
                let v = rc.borrow().iter().map(|x| x.deep_clone()).collect();
                Value::Set(Rc::new(RefCell::new(v)))
            }
            Value::Dict(rc) => {
                let b = rc.borrow();
                let mut d = DictData::new(b.key_type.clone(), b.item_type.clone());
                for (k, v) in b.iter() {
                    let key_val = match k {
                        DictKey::Int(n) => Value::Int(*n),
                        DictKey::Str(s) => Value::Str(Rc::from(&**s)),
                        DictKey::Bool(b) => Value::Bool(*b),
                        DictKey::None => Value::None,
                    };
                    d.set(key_val, v.deep_clone());
                }
                Value::Dict(Rc::new(RefCell::new(d)))
            }
            Value::Tuple(rc) => {
                let vals = rc.all_values().iter().map(|x| x.deep_clone()).collect();
                Value::Tuple(Rc::new(TupleData::new(vals, rc.all_types().to_vec())))
            }
            Value::Slice(s) => Value::Slice(Rc::new(SliceValue {
                begin: s.begin.as_ref().map(|v| v.deep_clone()),
                end: s.end.as_ref().map(|v| v.deep_clone()),
                step: s.step.as_ref().map(|v| v.deep_clone()),
            })),
            Value::Instance(rc) => {
                let b = rc.borrow();
                let boxed_fields: Vec<Option<(Value, bool)>> = b
                    .boxed_fields
                    .iter()
                    .map(|slot| slot.as_ref().map(|(v, m)| (v.deep_clone(), *m)))
                    .collect();
                // raw ブロックは POD なので clone = memcpy（class_id/flags/raw フィールドすべて保持）
                Value::Instance(Rc::new(RefCell::new(InstanceData {
                    raw: b.raw.clone(),
                    class: Rc::new(b.class.deep_clone()),
                    boxed_fields,
                })))
            }
            Value::Function(rc) => Value::Function(Rc::new(FnValue {
                name: rc.name.clone(),
                params: rc.params.clone(),
                // ⚠ **`Rc` を clone してはいけない**（#45）。`Rc<[Stmt]>` の参照カウントは
                // 非アトミックなので、スレッドへ送る複製で共有すると親と競合する（#15）。
                // 中身を複製して**独立した Rc** を作る。
                body: std::rc::Rc::from(&rc.body[..]),
                is_python: rc.is_python,
                captured_env: deep_clone_captured_env(&rc.captured_env),
            return_type: None,
            // ⚠ スレッドへ送る複製では定義サイトの `Rc` を持ち出さない（#15/#30）。
            vm_chunk: None,
            })),
            Value::OverloadedFn(fns) => Value::OverloadedFn(
                fns.iter()
                    .map(|rc| {
                        Rc::new(FnValue {
                            name: rc.name.clone(),
                            params: rc.params.clone(),
                            // ⚠ **`Rc` を clone してはいけない**（#45/#15）。上と同じ理由。
                            body: std::rc::Rc::from(&rc.body[..]),
                            is_python: rc.is_python,
                            captured_env: deep_clone_captured_env(&rc.captured_env),
                            return_type: rc.return_type.clone(),
                            // ⚠ スレッドへ送る複製では定義サイトの `Rc` を持ち出さない（#15/#30）。
                            vm_chunk: None,
                        })
                    })
                    .collect(),
            ),
            Value::GeneratorFn(rc) => Value::GeneratorFn(Rc::new(GeneratorFnValue {
                name: rc.name.clone(),
                params: rc.params.clone(),
                body: rc.body.clone(),
                captured_env: deep_clone_captured_env(&rc.captured_env),
            })),
            Value::Generator(rc) => {
                let b = rc.borrow();
                let vals = b.values.iter().map(|v| v.deep_clone()).collect();
                Value::Generator(Rc::new(RefCell::new(GeneratorState {
                    values: vals,
                    index: b.index,
                })))
            }
            Value::Class(rc) => Value::Class(Rc::new(rc.deep_clone())),
            Value::Namespace(rc) => {
                let members = rc
                    .members
                    .iter()
                    .map(|(k, v)| (k.clone(), v.deep_clone()))
                    .collect();
                Value::Namespace(Rc::new(NamespaceData {
                    name: rc.name.clone(),
                    members,
                }))
            }
            // TemplateFn / TemplateClass / TemplateGenFn contain only Clone data (no RefCell)
            Value::TemplateFn(rc) => Value::TemplateFn(rc.clone()),
            Value::TemplateClass(rc) => Value::TemplateClass(rc.clone()),
            Value::TemplateGenFn(rc) => Value::TemplateGenFn(rc.clone()),
            // Arc-wrapped types: atomic refcount, safe to share across threads
            Value::PyObject(arc) => Value::PyObject(Arc::clone(arc)),
            Value::NativeFunction(arc) => Value::NativeFunction(Arc::clone(arc)),
            Value::ResultVal { ok, inner } => Value::ResultVal {
                ok: *ok,
                inner: Box::new(inner.deep_clone()),
            },
            // Primitive type tags, async values — just clone
            other => other.clone(),
        }
    }
}


// ---------------------------------------------------------------------------
// Control-flow signals
// ---------------------------------------------------------------------------

/// ツリーウォーク（`exec()`）が返す制御フロー信号。
///
/// ⚠ **#33 でほぼ退化した**。`Break`/`Continue`/`Return`/`BlockReturn`/`BlockYield` は
/// すべて削除済みで、制御フローは**バイトコード VM がジャンプで表現する**。
/// ツリーウォークが実行するのは定義文だけ（#10-d）なので、実際に流れるのは
/// `Normal`（定義文の完了）と `Raise`（`exec_raise` の 1 箇所）の 2 つだけ。
/// （`Return` は #51 で削除 — 構築も match も 0 件だった。）
#[derive(Debug)]
pub enum ExecResult {
    /// 通常終了。次の文へ制御を移す。
    Normal,
    /// コールスタックを遡って伝播中の言語レベル例外。`try/except` で捕捉される。
    Raise(RaisedError),
}

#[cfg(test)]
mod size_tests {
    use super::Value;

    /// `Value` は 32 バイトに収まっていること（§7.4-2 で 72→32B に縮めた成果を守る）。
    /// #15 で `Str` を `String`(24B) → `Rc<str>`(16B) にしたが、最大変種は別なので不変。
    #[test]
    fn value_stays_32_bytes() {
        assert_eq!(std::mem::size_of::<Value>(), 32);
    }
}
