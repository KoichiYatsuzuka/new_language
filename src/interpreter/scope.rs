// scope.rs — スコープ管理 (push_scope / switch_globals / get_var / declare_var / assign_var)
//
// `Interpreter` のスコープスタック（`Vec<HashMap<String, Var>>`）を操作するメソッド群。
// インデックス 0 がグローバルスコープ、末尾がローカルスコープ（最内部）。
// 変数の検索は末尾（最内部）から先頭（グローバル）へ向かって行われる（レキシカルスコープ規則）。


use super::{Interpreter, Value, Var};

impl Interpreter {
    /// 組み込みだけを入れた大域（メインの大域と、各モジュールの大域の雛形）。
    ///
    /// ⚠ 組み込みの並びは**ここ 1 か所**。メインとモジュールで食い違うと、モジュールの中でだけ
    ///   組み込みが見えなくなる。
    pub(super) fn builtin_global_scope(event_loop: &Value) -> super::ScopeMap {
        let mut global = super::ScopeMap::default();
        super::built_in_types::register_builtin_globals(&mut global);
        // Signal: テンプレート型コンストラクタ。Signal[T]() で Value::Signal を生成する。
        global.insert(
            "Signal".to_string(),
            Var::new(Value::Type("Signal".to_string()), false),
        );
        // EventLoop は単一の値（どの大域からも同じものが見える）。
        global.insert("EventLoop".to_string(), Var::new(event_loop.clone(), false));
        global
    }

    /// 今の大域（`scopes[0]`）を `to` の大域へ差し替え、差し替える前の添字を返す
    /// （名前空間の分離・2026-09-26）。
    ///
    /// ⚠ 呼び出し側は**必ず戻す**（`let prev = self.switch_globals(g); …; self.switch_globals(prev);`）。
    ///   失敗（`Err`）で抜けるときも戻すこと。
    /// ⚠ slot キャッシュ（`slot_epoch` で焼いた大域の slot 番号）は進めない。コードは常に自分を
    ///   定義したモジュールの大域で走る（関数値が大域を持ち、呼び出しで差し替える）ので、
    ///   あるコードのキャッシュが別の大域に当たることは無い。
    #[inline]
    pub(crate) fn switch_globals(&mut self, to: u32) -> u32 {
        let prev = self.cur_globals;
        if to != prev {
            self.switch_globals_slow(to);
        }
        prev
    }

    #[inline(never)]
    fn switch_globals_slow(&mut self, to: u32) {
        // ⚠ 知らない添字（非同期のワーカーへ大域を渡しそこねた等）は差し替えない。
        if to as usize >= self.global_scopes.len() {
            return;
        }
        let cur = self.cur_globals as usize;
        // 今の大域を置き場へ戻し、行き先の大域を `scopes[0]` へ持ってくる。
        std::mem::swap(&mut self.scopes[0], &mut self.global_scopes[cur]);
        std::mem::swap(&mut self.scopes[0], &mut self.global_scopes[to as usize]);
        self.cur_globals = to;
    }

    /// モジュールの本体を走らせる枠に入る（名前空間の分離・2026-09-26）。
    ///
    /// 新しい大域（組み込みだけ）を作って `scopes[0]` に据え、**呼び出し側のローカルを隠す**
    /// （関数やブロックの中の `import` でも、モジュールの本体は呼び出し側の名前を見ない）。
    /// 戻り値は [`Self::leave_module_frame`] に渡す。
    pub(super) fn enter_module_frame(&mut self, module: &[String]) -> (u32, Vec<super::ScopeMap>, usize) {
        let fresh = Self::builtin_global_scope(&self.event_loop_value);
        self.global_scopes.push(fresh);
        // このモジュールで定義するクラスの `module_name`（10-8）。
        self.global_module_names
            .push((!module.is_empty()).then(|| std::rc::Rc::from(module.join(".").as_str())));
        let id = (self.global_scopes.len() - 1) as u32;
        let locals = self.scopes.split_off(1);
        let floor = std::mem::replace(&mut self.frame_floor, 1);
        let prev = self.switch_globals(id);
        (prev, locals, floor)
    }

    /// 今の大域（モジュールの大域）から、名前空間に出す名前と値を取り出す。
    ///
    /// ⚠ 組み込み（`int` / `EventLoop` …）は出さない（大域には雛形として入っているだけ）。
    ///   モジュールが同じ名前を宣言し直したもの（`declared` にあるもの）は出す。
    pub(crate) fn module_members(
        &self,
        declared: &std::collections::HashMap<String, bool>,
    ) -> std::collections::HashMap<String, Value> {
        let builtins = Self::builtin_global_scope(&self.event_loop_value);
        self.scopes[0]
            .iter()
            .filter(|(n, _)| !builtins.contains_key(n.as_str()) || declared.contains_key(n.as_str()))
            .map(|(k, v)| (k.clone(), v.get_value()))
            .collect()
    }

    /// [`Self::enter_module_frame`] で入った枠を出る。モジュールの大域は置き場に残る
    /// （そのモジュールの関数が後で呼ばれたときに使う）。
    pub(super) fn leave_module_frame(&mut self, saved: (u32, Vec<super::ScopeMap>, usize)) {
        let (prev, locals, floor) = saved;
        // ⚠ 本体の途中で失敗して積んだスコープが残っていれば捨てる。
        self.scopes.truncate(1);
        self.switch_globals(prev);
        self.scopes.extend(locals);
        self.frame_floor = floor;
    }

    /// 非同期タスクへ送る各モジュールの大域の複製と、今の大域の添字（`async_mgr::TaskEnv`）。
    pub(crate) fn snapshot_globals_for_task(&self) -> (Vec<Vec<(String, Value, bool)>>, u32) {
        let cur = self.cur_globals as usize;
        let globals = (0..self.global_scopes.len())
            .map(|i| {
                // ⚠ 今の大域は `scopes[0]` にある（置き場のその添字は空の置き物）。
                let scope = if i == cur { &self.scopes[0] } else { &self.global_scopes[i] };
                scope
                    .iter()
                    .map(|(n, v)| (n.clone(), super::async_mgr::clone_for_task(v), v.is_mutable()))
                    .collect()
            })
            .collect();
        (globals, self.cur_globals)
    }

    /// 非同期タスクのワーカーに、送られてきた各モジュールの大域を**同じ添字で**置き、
    /// タスクを出したコードの大域へ切り替える（`async_mgr::TaskEnv`）。
    pub(crate) fn install_task_globals(&mut self, globals: Vec<Vec<(String, Value, bool)>>, cur: u32) {
        debug_assert_eq!(self.cur_globals, 0, "a fresh worker starts in its main globals");
        for (i, entries) in globals.into_iter().enumerate() {
            if i == 0 {
                for (n, v, m) in entries {
                    self.scopes[0].insert(n, Var::new(v, m));
                }
            } else {
                let mut g = Self::builtin_global_scope(&self.event_loop_value);
                for (n, v, m) in entries {
                    g.insert(n, Var::new(v, m));
                }
                self.global_scopes.push(g);
                // ⚠ 添字を揃えるだけ（ワーカーでクラスを定義することは無い。送られてきたクラスは
                //   `module_name` を自分で持っている）。
                self.global_module_names.push(None);
            }
        }
        // ⚠ 具体化のキャッシュを組み直す（D36・タスク 2-16）。キャッシュの鍵はテンプレートの値の
        //   同一性で、ワーカーのテンプレートは複製なので、親の登録はそのままでは使えない。
        //   各大域の具体化（`Box[int]`）を、同じ大域のテンプレートに結び付け直す。
        for g in 0..self.global_scopes.len() as u32 {
            self.switch_globals(g);
            let instances: Vec<(String, Value)> = self.scopes[0]
                .iter()
                .filter(|(n, _)| n.ends_with(']'))
                .map(|(n, v)| (n.clone(), v.get_value()))
                .collect();
            for (name, value) in instances {
                self.register_mono_instance(&name, &value);
            }
        }
        self.switch_globals(cur);
    }

    /// 新しいローカルスコープをスタックに積む。
    /// ブロック・関数・if/while/for の実行開始時に呼ぶ。
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(Default::default());
    }


    /// 指定名の変数エントリ（`Var`）を内側スコープから外側スコープへ向けて検索する。
    ///
    /// - `name`: 検索する変数名
    ///
    /// 戻り値: 見つかった `Var` への参照、存在しない場合は `None`
    pub(super) fn get_var(&self, name: &str) -> Option<&Var> {
        // 現関数のローカル（frame_floor..）を最内部から外側へ検索し、なければグローバル（0）を見る。
        // 呼び出し元のローカル（1..frame_floor）はレキシカル隔離のため走査しない。
        for scope in self.scopes[self.frame_floor..].iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        self.scopes[0].get(name)
    }

    /// 組み込み名がユーザーの束縛でシャドウされているか（#15d）。
    ///
    /// 呼び先の振り分けが「名前が組み込みと一致するか」だけを見ていると、
    /// `let repr = my_fn` としても組み込みが横取りする。`Resolution` は使えない —
    /// リゾルバが処理するのはトップレベル関数の本体だけで、**モジュール最上位・
    /// テンプレート本体・合成 AST は常に `Unresolved`** だからである
    /// （`Unresolved` は「シャドウが無い」を意味しない）。よって実際の束縛を見る。
    ///
    /// ⚠ `Value::Type(name)` は**組み込み登録そのもの**なのでシャドウとみなさない。
    /// `register_builtin_globals` が `len` を型値としてグローバルに置いており
    /// （ネイティブの `cb_get_global("len")` 用）、これを除かないと `len()` が
    /// 組み込み経路から `call_type_by_name_evaled` へ逸れる。そちらには
    /// `Value::PyObject` のアームが無いので **`len(py_obj)` が壊れ**、
    /// 組み込み経路を保つ VM 側とも食い違う。
    pub(crate) fn builtin_is_shadowed(&self, name: &str) -> bool {
        match self.get_var(name) {
            None => false,
            Some(Var::Immutable(Value::Type(t)) | Var::Mutable(Value::Type(t))) => t != name,
            Some(_) => true,
        }
    }

    /// グローバルスコープ側だけを見た同判定（VM 用・#15d）。
    /// VM はローカルのシャドウをコンパイル時に `slots.contains_key` で除外済みなので、
    /// 実行時に見る必要があるのはグローバルだけ。
    pub(crate) fn builtin_is_shadowed_global(&self, name: &str) -> bool {
        match self.scopes[0].get(name) {
            None => false,
            Some(Var::Immutable(Value::Type(t)) | Var::Mutable(Value::Type(t))) => t != name,
            Some(_) => true,
        }
    }

    /// 指定名の変数の値だけをクローンして返す。
    /// セル（クロージャキャプチャ）がある場合はセルの値を返す。
    /// 変数が存在しない場合は `None`。
    pub(super) fn get_val(&self, name: &str) -> Option<Value> {
        self.get_var(name).map(|v| v.get_value())
    }

    /// VM デバッガ: 停止スコープから名前引きで値を取る（`LoadName` op）。
    pub(crate) fn vm_load_name(&self, name: &str) -> Option<Value> {
        self.get_val(name)
    }

    /// VM デバッガ: `let dbg::name = expr` を停止スコープへ宣言する（`DeclareName` op）。
    /// 非識別子ソースの `let` 意味論に合わせ、Instance は deep_copy + freeze する。
    pub(crate) fn vm_declare_debug(&mut self, name: &str, value: Value) -> Result<(), String> {
        let v = if matches!(value, Value::Instance(_)) {
            let copied = Self::deep_copy_value(value);
            self.apply_freeze_to_value(&copied, true)?;
            copied
        } else {
            value
        };
        self.declare_var(name.to_string(), Var::new(v, false));
        Ok(())
    }

    /// 最内部スコープに新しい変数を宣言する。
    /// 同名の変数が同スコープ内に既に存在する場合は上書きされる。
    ///
    /// - `name`: 変数名
    /// - `var`: 値と可変フラグを含む `Var`
    ///
    /// パニック: スコープスタックが空のとき（`new()` 後は発生しない）
    pub(super) fn declare_var(&mut self, name: String, var: Var) {
        self.scopes.last_mut().unwrap().insert(name, var);
    }

    /// 既存の変数に新しい値を代入する。内側スコープから外側スコープへ向けて変数を検索する。
    ///
    /// - `name`: 代入先の変数名
    /// - `value`: 新しい値
    ///
    /// 戻り値: `Ok(())` — 成功。`Err(message)` — 変数未定義 (`NameError`) または不変変数 (`TypeError`)
    pub(super) fn assign_var(&mut self, name: &str, value: Value) -> Result<(), String> {
        // 現関数のローカル（frame_floor..）を内側から検索し、なければグローバル（0）。
        let floor = self.frame_floor;
        for scope in self.scopes[floor..].iter_mut().rev() {
            if let Some(v) = scope.get_mut(name) {
                if !v.is_mutable() {
                    return Err(format!(
                        "TypeError: cannot assign to immutable variable '{name}'"
                    ));
                }
                v.set_value(value);
                return Ok(());
            }
        }
        if let Some(v) = self.scopes[0].get_mut(name) {
            if !v.is_mutable() {
                return Err(format!(
                    "TypeError: cannot assign to immutable variable '{name}'"
                ));
            }
            v.set_value(value);
            return Ok(());
        }
        Err(format!("NameError: '{name}' is not defined"))
    }

    /// 変数を不変（`Immutable`）に変更する（`freeze` 文で使用）。
    /// `Cell` 変数（クロージャにキャプチャ済み）は freeze できない。
    /// `SlotCell`（スロットキャッシュ昇格済み）は値スナップショットで `Immutable` に戻し、
    /// `slot_epoch` を進めて全 AST スロットキャッシュを無効化する。
    pub(super) fn make_var_immutable(&mut self, name: &str) {
        // 対象スコープの index を先に確定する（現関数のローカル frame_floor.. を内側から、なければグローバル 0）。
        let floor = self.frame_floor;
        let mut idx: Option<usize> = None;
        for i in (floor..self.scopes.len()).rev() {
            if self.scopes[i].contains_key(name) {
                idx = Some(i);
                break;
            }
        }
        let idx = match idx {
            Some(i) => i,
            None if self.scopes[0].contains_key(name) => 0,
            None => return,
        };
        let mut freeze_slot = false;
        if let Some(v) = self.scopes[idx].get_mut(name) {
            match v {
                Var::Mutable(val) => {
                    *v = Var::Immutable(std::mem::replace(val, Value::None));
                }
                Var::SlotCell(rc) => {
                    let snapshot = rc.borrow().clone();
                    *v = Var::Immutable(snapshot);
                    freeze_slot = true;
                }
                _ => {}
            }
        }
        if freeze_slot {
            self.slot_epoch += 1;
        }
    }
}
