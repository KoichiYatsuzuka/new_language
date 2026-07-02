// scope.rs — スコープ管理 (push_scope / pop_scope / get_var / declare_var / assign_var)
//
// `Interpreter` のスコープスタック（`Vec<HashMap<String, Var>>`）を操作するメソッド群。
// インデックス 0 がグローバルスコープ、末尾がローカルスコープ（最内部）。
// 変数の検索は末尾（最内部）から先頭（グローバル）へ向かって行われる（レキシカルスコープ規則）。

use std::collections::HashMap;

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
        // 末尾（最内部スコープ）から先頭（グローバル）へ向けて順に検索する
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v);
            }
        }
        None
    }

    /// 指定名の変数の値だけをクローンして返す。
    /// セル（クロージャキャプチャ）がある場合はセルの値を返す。
    /// 変数が存在しない場合は `None`。
    pub(super) fn get_val(&self, name: &str) -> Option<Value> {
        self.get_var(name).map(|v| v.get_value())
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
        for scope in self.scopes.iter_mut().rev() {
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
        Err(format!("NameError: '{name}' is not defined"))
    }

    /// 変数を不変（`Immutable`）に変更する（`freeze` 文で使用）。
    /// `Cell` 変数（クロージャにキャプチャ済み）は freeze できない。
    /// `SlotCell`（スロットキャッシュ昇格済み）は値スナップショットで `Immutable` に戻し、
    /// `slot_epoch` を進めて全 AST スロットキャッシュを無効化する。
    pub(super) fn make_var_immutable(&mut self, name: &str) {
        let mut freeze_slot = false;
        for scope in self.scopes.iter_mut().rev() {
            if let Some(v) = scope.get_mut(name) {
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
                break;
            }
        }
        if freeze_slot {
            self.slot_epoch += 1;
        }
    }
}
