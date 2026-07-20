// classes/lookup.rs — 継承チェーン探索と値コピー: lookup_method_in_class / lookup_class_var / copy_value。

use {
    std::rc::Rc,
    crate::interpreter::{
        ClassValue, FnValue,
        Interpreter, Value,
    },
};

impl Interpreter {
    /// メソッドをクラスから検索する。クラス本体の `methods` マップのみを参照する。
    ///
    /// 注意: クラス間継承は無効化されており、trait ベースの継承のみパース時にサポートされる。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `method_name`: 検索するメソッド名
    ///
    /// 戻り値: `Some(Vec<Rc<FnValue>>)` — オーバーロード候補リスト。`None` — 見つからない
    pub(crate) fn lookup_method_in_class(
        &self,
        class: &Rc<ClassValue>,
        method_name: &str,
    ) -> Option<Vec<Rc<FnValue>>> {
        if let Some(overloads) = class.methods.get(method_name) {
            return Some(overloads.clone());
        }
        // クラス間継承は無効。trait ベースの継承はパース時にのみサポートされる。
        None
    }

    /// `const` クラス変数をクラスの `class_vars` から検索する。
    ///
    /// 現在はクラス変数の継承（基底クラスへの遡及検索）は未実装。
    ///
    /// - `class`: 検索対象のクラス定義
    /// - `name`: クラス変数名
    ///
    /// 戻り値: `Some(Value)` — クラス変数の値。`None` — 見つからない
    pub(crate) fn lookup_class_var(class: &Rc<ClassValue>, name: &str) -> Option<Value> {
        class.class_vars.get(name).cloned()
        // 注: 基底クラスへの遡及検索にはスコープへのアクセスが必要なため、現在は未実装
    }

    /// インスタンス値をコピーする。
    ///
    /// 優先順位:
    /// 1. インスタンスのクラスに `__copy__` メソッドが定義されており、引数なし（`self` のみ）で
    ///    呼び出せるオーバーロードがあれば、それを呼び出す。
    /// 2. `__copy__` が存在しないか引数なしで呼び出せるオーバーロードがなければ、
    ///    `deep_copy_unfrozen` によるデフォルトのディープコピーを実行する。
    ///    `let` バインドのフリーズを解除し、新鮮な可変インスタンスとして返す。
    ///
    /// インスタンス以外の値（List / Dict 等）は `deep_copy_unfrozen` を使用する。
    ///
    /// メモリ不足などでコピーがパニックした場合は `MemoryError` を返す。
    pub(crate) fn copy_value(&mut self, val: Value) -> Result<Value, String> {
        if let Value::Instance(ref inst_rc) = val {
            let class = inst_rc.borrow().class.clone();
            if let Some(overloads) = self.lookup_method_in_class(&class, "__copy__") {
                // 引数なし（self のみ）で呼び出せるオーバーロードを選別する
                let callable: Vec<Rc<FnValue>> = overloads
                    .into_iter()
                    .filter(|f| {
                        f.params
                            .iter()
                            .filter(|p| p.name != "self")
                            .all(|p| p.default.is_some() || p.variadic)
                    })
                    .collect();
                if !callable.is_empty() {
                    return if callable.len() == 1 {
                        self.exec_fn(callable[0].clone(), &[], Some(val), "__copy__", None)
                    } else {
                        self.dispatch_overload(callable, &[], Some(val), None)
                    };
                }
            }
        }
        // デフォルト: フリーズ解除ディープコピー（パニック=メモリ不足を RuntimeError に変換）
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Interpreter::deep_copy_unfrozen(val)
        }))
        .map_err(|_| "MemoryError: insufficient memory for copy".to_string())
    }
}
