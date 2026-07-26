// scope.rs — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)
//
// `Interpreter` のスコープスタック（`Vec<HashMap<String, Var>>`）を操作するメソッド群。
// インデックス 0 がグローバルスコープ、末尾がローカルスコープ（最内部）。
// 変数の検索は末尾（最内部）から先頭（グローバル）へ向かって行われる（レキシカルスコープ規則）。


use super::{Interpreter, Value, Var};

impl Interpreter {
    /// 新しいローカルスコープをスタックに積む。
    /// ブロック・関数・if/while/for の実行開始時に呼ぶ。
    pub(super) fn push_scope(&mut self) {
        self.scopes.push(Default::default());
    }

    /// 最内部のローカルスコープをスタックから取り除く。
    /// グローバルスコープ（インデックス 0）は削除しない。
    pub(super) fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
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
