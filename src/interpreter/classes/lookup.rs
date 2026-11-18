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
        self.copy_value_held(val, 0)
    }

    /// [`Self::copy_value`] の本体。`held` は、呼ぶ側が `val` のほかに持っていて、束縛が済むまで誰からも見えない
    /// 参照の数（評価済みの引数の並びの 1 個など）。
    ///
    /// 誰とも共有していない値（[`Self::is_unshared`]・作りたての `Vec3(...)` など）は写さずにそのまま渡す
    /// （copy_alloc_speed_plan.md 2-2）。写しと同じ印になるよう、フリーズはその場で解く（[`Self::unfreeze_in_place`]）。
    /// ⚠ `__copy__` を持つクラスの値は今までどおり `__copy__` を呼ぶ（利用者のコードが走る）。
    /// ⚠⚠ `held` を多く数えると、持ち主のいる値を写さずに渡してしまう（`let` の仮引数が外から書き換わる）。
    ///   確かに数えられる分だけを渡すこと。少なく数えるのは安全（写すだけ）。
    pub(crate) fn copy_value_held(&mut self, val: Value, held: usize) -> Result<Value, String> {
        let fresh = Self::is_unshared(&val, 1 + held) && !self.has_copy_hook(&val);
        crate::interpreter::copy_stats::record(crate::interpreter::copy_stats::CopyKind::LetParam, &val, fresh);
        if fresh {
            Self::unfreeze_in_place(&val);
            return Ok(val);
        }
        self.copy_value_uncounted(val)
    }

    /// `val` がインスタンスで、そのクラスが `__copy__` を持つか（[`Self::copy_value_uncounted`] が呼ぶもの）。
    pub(crate) fn has_copy_hook(&self, val: &Value) -> bool {
        match val {
            Value::Instance(rc) => {
                // ⚠ 以前は `lookup_method_in_class` が候補の `Vec` を写していた（呼び出しのたびに確保・6-3）。
                //   その後も名前で引いていた（`contains_key`）のを、クラスごとの印にした（7-1）
                rc.borrow().class.has_copy_method()
            }
            _ => false,
        }
    }

    /// [`Self::copy_value`] の本体（診断 `copy_stats` に数えない。利用者が書いた `x.copy()` は別の種類として数える）。
    pub(crate) fn copy_value_uncounted(&mut self, val: Value) -> Result<Value, String> {
        // `__copy__` を持たないクラス（ほとんど）は名前で引かない（クラスごとの印・7-1）
        let hooked = match &val {
            Value::Instance(inst_rc) => {
                let inst = inst_rc.borrow();
                inst.class.has_copy_method().then(|| inst.class.clone())
            }
            _ => None,
        };
        if let Some(class) = hooked {
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
