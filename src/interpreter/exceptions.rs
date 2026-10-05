// exceptions.rs — 例外クラス構築・トレースバック
// (get_context_lines / exc_matches / make_internal_raised_error)
//
// トレースバック表示のためのソースコンテキスト抽出・例外マッチング判定・
// インタープリタ内部エラーを言語例外へ変換するユーティリティを提供する。
//
// 注: 標準例外クラスの構築 (make_error_class) は built_in_types.rs に移動した。

use std::cell::RefCell;
use std::rc::Rc;

use super::{ClassValue, InstanceData, Interpreter, RaisedError, Value};

impl Interpreter {
    /// ソースマップから指定行の前後 `n` 行以内のソースコンテキストを返す。
    ///
    /// - `file`: ソースファイル名（`source_map` のキーと一致させること）
    /// - `line`: 中心とする行番号（1始まり）
    /// - `n`: 取得する行数の上限（中心行の前後を含む）
    ///
    /// 戻り値: 該当行が存在する場合は複数行の文字列（`\n` 区切り）、存在しない場合は空文字列
    pub(super) fn get_context_lines(&self, file: &str, line: usize, n: usize) -> String {
        let lines = match self.source_map.get(file) {
            Some(l) => l,
            None => return String::new(),
        };
        if line == 0 || lines.is_empty() {
            return String::new();
        }
        let half = n / 2;
        // 0始まりインデックスに変換してパディング付きで範囲を決定する
        let start = line.saturating_sub(half + 1);
        let end = (line + half).min(lines.len());
        lines[start..end].join("\n")
    }

    /// インタープリタ内部の `Err("ClassName: message")` 文字列を `RaisedError` に変換する。
    ///
    /// 既知の例外クラス名で始まる文字列のみ変換する。マッチしない場合は `None`。
    /// これにより `try/except` がインタープリタ内部エラーを捕捉できるようになる。
    pub(super) fn make_internal_raised_error(&mut self, msg: &str) -> Option<RaisedError> {
        // 表は組み込みの例外クラスの表 1 つ（`type_check::names::BUILTIN_EXCEPTIONS`・タスク 4-1）。
        // ⚠ 以前は手で揃えた別の一覧で、`RecursionError` が抜けて `except RecursionError:` で
        //   捕まえられなかった（実測）。
        // ⚠ `SyntaxError` の系統は変換しない。内部で `SyntaxError: 'break' outside for/while loop` などを
        //   返すが、CPython ではコンパイル時の誤りで `try` では捕まらない（`except Exception` に黙って
        //   飲まれないようにする）。
        let mut catchable = crate::type_check::names::BUILTIN_EXCEPTIONS
            .iter()
            .map(|(n, _)| *n)
            .filter(|n| !matches!(*n, "SyntaxError" | "IndentationError" | "TabError"));

        let class_name = catchable.find(|&cn| {
            msg.starts_with(cn)
                && (msg.len() == cn.len()
                    || msg.as_bytes().get(cn.len()).copied() == Some(b':')
                    || msg.as_bytes().get(cn.len()).copied() == Some(b' '))
        })?;

        let message = msg[class_name.len()..]
            .trim_start_matches(':')
            .trim()
            .to_string();

        let cls = match self.get_val(class_name) {
            Some(Value::Class(c)) => c,
            _ => return None,
        };

        // `args` は文言 1 つの組（空なら空の組・タスク 4-2）。⚠ CPython は `KeyError` ならキーそのもの、
        // `OSError` なら `(errno, strerror)` だが、内部のエラーは文言しか持たない。
        let args = self.tuple_of(if message.is_empty() { Vec::new() } else { vec![Value::str(message.clone())] });

        // フィールドレイアウト: message=0, code_context=1, file=2, line=3, col=4, args=5
        // (make_error_class の field_index と対応)
        let args_idx = cls.field_index.get("args").copied();
        let mut data = InstanceData::new_empty(cls, 0);
        data.store_field(0, Value::str(message), false); // message
        data.store_field(1, Value::str(String::new()), false); // code_context
        data.store_field(2, Value::str(String::new()), false); // file
        data.store_field(3, Value::Int(0), false); // line
        data.store_field(4, Value::Int(0), false); // col
        if let Some(idx) = args_idx {
            data.store_field(idx, args, false); // args
        }
        let inst = Value::Instance(Rc::new(RefCell::new(data)));

        Some(RaisedError {
            exception: inst,
            frames: vec![],
        })
    }

    /// 例外インスタンスのクラスが `except` 節の型名にマッチするか判定する。
    ///
    /// マッチ条件: クラス名が `type_name` と一致するか、または `bases`（基底クラス・trait）に含まれる場合。
    /// `Exception` はすべての例外にマッチする（[`ClassValue::is_a`]）。
    ///
    /// - `inst_class`: 例外インスタンスのクラス定義
    /// - `type_name`: `except` 節で指定された型名
    ///
    /// 戻り値: `true` — マッチあり（例外がこの handler で捕捉される）
    pub(super) fn exc_matches(inst_class: &Rc<ClassValue>, type_name: &str) -> bool {
        // `except Exception` はすべての例外を捕まえる（`ClassValue::is_a`・フェーズ10 10-19）。
        inst_class.is_a(type_name)
    }

    /// 組み込みの例外の階層のクラスか（`BaseException` とその子孫と、それを継承した Python のクラス）。
    ///
    /// これらは `args` を持ち、`str(e)` / `repr(e)` が CPython と同じになる（タスク 4-2）。
    /// ⚠ `Error` を実装しただけの Arrow のクラス（`class MyErr(Error)`）は含まない（`args` を持たない）。
    pub(crate) fn in_exception_hierarchy(cls: &ClassValue) -> bool {
        cls.name == "BaseException" || cls.bases.iter().any(|b| b == "BaseException")
    }

    /// 例外の `args` と `message`（CPython の `str(e)`）を入れる（タスク 4-2）。
    ///
    /// `str(e)` は CPython の `BaseException_str` / `KeyError_str` と同じ: 引数が無ければ `''`、
    /// 1 つならその `str`（`KeyError` だけは `repr`）、2 つ以上なら `args` の組の `repr`。
    /// ⚠ 可変（`true`）で入れる。Python の `__init__` が `self.message = ..` と書いても通るように
    ///   （CPython の例外の属性は書き換えられる）。Arrow のソースからの書き換えは型検査が `let` として止める。
    pub(crate) fn exc_set_args(
        &mut self,
        inst: &Rc<RefCell<InstanceData>>,
        args: Vec<Value>,
    ) -> Result<(), String> {
        let cls = inst.borrow().class.clone();
        let message = match args.as_slice() {
            [] => String::new(),
            [one] if cls.is_a("KeyError") => self.repr_val(one)?,
            [one] => self.display_str(one)?,
            _ => {
                let t = self.tuple_of(args.clone());
                self.repr_val(&t)?
            }
        };
        let args_tuple = self.tuple_of(args);
        let mut data = inst.borrow_mut();
        if let Some(&idx) = cls.field_index.get("message") {
            data.store_field(idx, Value::str(message), true);
        }
        if let Some(&idx) = cls.field_index.get("args") {
            data.store_field(idx, args_tuple, true);
        }
        Ok(())
    }

    /// 例外を作る・`super().__init__(..)` の引数から `args` を取る。キーワード引数は受けない（CPython と同じ文言）。
    pub(crate) fn exc_positional_args(
        cls: &ClassValue,
        evaled: &[(Option<String>, Value, bool)],
    ) -> Result<Vec<Value>, String> {
        if evaled.iter().any(|(k, _, _)| k.is_some()) {
            return Err(format!("TypeError: {}() takes no keyword arguments", cls.name));
        }
        Ok(evaled.iter().map(|(_, v, _)| v.clone()).collect())
    }

    /// 値の列を組にする（要素の型名は実行時の名前）。
    fn tuple_of(&self, values: Vec<Value>) -> Value {
        let types = values.iter().map(|v| self.type_name(v).to_string()).collect();
        Value::Tuple(Rc::new(crate::interpreter::TupleData::new(values, types)))
    }
}
