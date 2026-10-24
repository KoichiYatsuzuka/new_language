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

}

/// `OSError(errno, strerror[, filename[, winerror[, filename2]]])` を読んだもの（[`Interpreter::exc_set_args`]・
/// known_bugs.md の #9）。`args` は例外の `args` に入れる組（`filename` があれば最初の 2 つ）。
struct OsErrorArgs {
    errno: Value,
    strerror: Value,
    filename: Value,
    filename2: Value,
    args: Vec<Value>,
}

impl Interpreter {
    /// 実行時の誤りの文言 `msg` に、例外にしたときの `args` を添えて `msg` を返す（known_bugs.md の #9）。
    /// 誤りは今までどおり文字列で運ぶ（位置・トレースバックは変わらない）。`make_internal_raised_error` が文言の
    /// 一致を確かめてから使う（[`Interpreter::error_args`]）。
    pub(crate) fn error_with_args(&mut self, msg: String, args: Vec<Value>) -> String {
        self.error_args = Some((msg.clone(), args));
        msg
    }

    /// 見つからないキーの `KeyError`（`d[k]` / `del d[k]` / `d.pop(k)` / `s.remove(x)` / `"{k}".format()`）。文言は
    /// CPython と同じくキーの `repr`、`args` はキーそのもの（`e.args[0] is k`・known_bugs.md の #9）。
    pub(crate) fn key_error(&mut self, key: &Value) -> String {
        let msg = format!("KeyError: {}", self.display_repr(key));
        self.error_with_args(msg, vec![key.clone()])
    }

    /// 文言を引数にする `KeyError`（`d.popitem()` / `s.pop()` が空のとき）。CPython の `args` は文言の文字列
    /// （`KeyError('popitem(): dictionary is empty')`）で、`str(e)` はその `repr`。
    pub(crate) fn key_error_text(&mut self, text: &str) -> String {
        let key = Value::str(text);
        self.key_error(&key)
    }

    /// インタープリタ内部の `Err("ClassName: message")` 文字列を `RaisedError` に変換する。
    ///
    /// 既知の例外クラス名で始まる文字列のみ変換する。マッチしない場合は `None`。
    /// これにより `try/except` がインタープリタ内部エラーを捕捉できるようになる。
    pub(super) fn make_internal_raised_error(&mut self, msg: &str) -> Option<RaisedError> {
        // ⚠⚠ 組み込みの例外の表そのもので引く（`builtin_exception_of`・表は `type_check::names::BUILTIN_EXCEPTIONS`・
        //    python_builtins_plan.md のタスク 4-1 / ffi_value_exchange_plan.md 7-4）。`SyntaxError` の系統は変換しない。
        let class_name = super::built_in_types::builtin_exception_of(msg)?;

        let message = msg[class_name.len()..]
            .trim_start_matches(':')
            .trim()
            .to_string();

        let cls = match self.get_val(class_name) {
            Some(Value::Class(c)) => c,
            _ => return None,
        };

        // `args` は送出の場が添えた値（`error_with_args`・known_bugs.md の #9）。`KeyError` ならキーそのもの、`OSError`
        // の系統なら `(errno, strerror, filename)` を `OSError(..)` と同じに読む（`args` は `(errno, strerror)`・属性も
        // 入れる）。添えていなければ文言 1 つの組（空なら空の組・タスク 4-2）。⚠ 控えは文言が完全に一致するときだけ
        // 使い、どちらでも取り去る（別の誤りの控えを拾わない）。`str(e)`（`message`）はどちらでも文言のまま。
        let given = match self.error_args.take() {
            Some((m, args)) if m == msg => Some(args),
            _ => None,
        };
        // ⚠ 控えの `OSError` の引数は `(errno, strerror, filename)` だけ（`os_error`）なので、読み方の誤り
        //   （`winerror`）は起きない。
        let os = match &given {
            Some(args) => Self::os_error_args(&cls, args).ok().flatten(),
            None => None,
        };
        let args = match (given, &os) {
            (_, Some(os)) => os.args.clone(),
            (Some(args), None) => args,
            (None, None) => if message.is_empty() { Vec::new() } else { vec![Value::str(message.clone())] },
        };
        let args = self.tuple_of(args);

        // 既定の欄（`make_error_class` の `field_defaults`・`OSError` の系統は `errno` … が `None`）を入れてから、
        // 文言と `args` を入れる。フィールドレイアウト: message=0, code_context=1, file=2, line=3, col=4, args=5
        let args_idx = cls.field_index.get("args").copied();
        let mut data = InstanceData::new_empty(cls.clone(), 0);
        for (name, default, mutable) in &cls.field_defaults {
            if let Some(&idx) = cls.field_index.get(name.as_str()) {
                data.store_field(idx, default.clone(), *mutable);
            }
        }
        data.store_field(0, Value::str(message), false); // message
        if let Some(idx) = args_idx {
            data.store_field(idx, args, false); // args
        }
        if let Some(os) = &os {
            Self::store_os_error_attrs(&mut data, os);
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
        // `OSError` の系統の `OSError(errno, strerror[, filename])`（known_bugs.md の #9）。
        if let Some(os) = Self::os_error_args(&cls, &args)? {
            let message = self.os_error_message(&os)?;
            let mut data = inst.borrow_mut();
            Self::store_os_error_attrs(&mut data, &os);
            if let Some(&idx) = cls.field_index.get("message") {
                data.store_field(idx, Value::str(message), true);
            }
            if let Some(&idx) = cls.field_index.get("args") {
                data.store_field(idx, self.tuple_of(os.args), true);
            }
            return Ok(());
        }
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

    /// `OSError` の系統の引数の読み方（CPython の `oserror_parse_args` / `oserror_init`・known_bugs.md の #9）。
    /// 引数が 2〜5 個なら `errno` / `strerror` / `filename` / (Windows の `winerror`) / `filename2` の順。`args` は
    /// `filename` が `None` でなければ最初の 2 つに切り詰める。`OSError` の系統でない・引数が 0 / 1 / 6 個以上なら `None`
    /// （属性は `None` のまま、`args` と `str(e)` はほかの例外と同じ）。
    /// ⚠ `winerror`（4 つ目）は持たないので、`None` 以外を渡すと `TypeError`（黙って捨てない）。
    fn os_error_args(cls: &ClassValue, args: &[Value]) -> Result<Option<OsErrorArgs>, String> {
        if !cls.field_index.contains_key("errno") || !(2..=5).contains(&args.len()) {
            return Ok(None);
        }
        // ⚠ 文言は `OSError()`（`OSError(2, ..)` は先に `FileNotFoundError` へ振り分けてあるので、クラスの名前では
        //   書いた呼び出しと食い違う）。
        if args.get(3).is_some_and(|w| !matches!(w, Value::None)) {
            return Err("TypeError: OSError(): the `winerror` argument (the 4th) is not supported; pass None".to_string());
        }
        let filename = args.get(2).cloned().unwrap_or(Value::None);
        let kept = if matches!(filename, Value::None) { args.to_vec() } else { args[..2].to_vec() };
        Ok(Some(OsErrorArgs {
            errno: args[0].clone(),
            strerror: args[1].clone(),
            filename,
            filename2: args.get(4).cloned().unwrap_or(Value::None),
            args: kept,
        }))
    }

    /// `OSError` の `str(e)`（CPython の `OSError_str`）: `[Errno 2] No such file or directory: 'p'`。`filename` が
    /// `None` なら `[Errno 2] No such file or directory`、`filename2` があれば `: 'a' -> 'b'`。
    /// ⚠ 作るときに決める（`message`）。作った後で `e.errno = ..` と書き換えても `str(e)` は変わらない（CPython は変わる）。
    fn os_error_message(&mut self, os: &OsErrorArgs) -> Result<String, String> {
        let mut out = format!("[Errno {}] {}", self.display_str(&os.errno)?, self.display_str(&os.strerror)?);
        if !matches!(os.filename, Value::None) {
            out.push_str(&format!(": {}", self.repr_val(&os.filename)?));
            if !matches!(os.filename2, Value::None) {
                out.push_str(&format!(" -> {}", self.repr_val(&os.filename2)?));
            }
        }
        Ok(out)
    }

    /// `errno` / `strerror` / `filename` / `filename2` の欄へ入れる（[`OS_ERROR_FIELDS`] の順）。
    ///
    /// [`OS_ERROR_FIELDS`]: super::built_in_types::OS_ERROR_FIELDS
    fn store_os_error_attrs(data: &mut InstanceData, os: &OsErrorArgs) {
        let values = [&os.errno, &os.strerror, &os.filename, &os.filename2];
        for (name, v) in super::built_in_types::OS_ERROR_FIELDS.iter().zip(values) {
            if let Some(&idx) = data.class.field_index.get(*name) {
                data.store_field(idx, v.clone(), true);
            }
        }
    }

    /// `open` が失敗したときの `OSError`（CPython と同じ文言 `[Errno 2] No such file or directory: 'p'`）。`args` は
    /// `(errno, strerror)`、`filename` はパス（`error_with_args` で控える・known_bugs.md の #9）。
    pub(crate) fn os_error(&mut self, class: &str, errno: Value, strerror: &str, filename: &str) -> String {
        let os = OsErrorArgs {
            errno,
            strerror: Value::str(strerror),
            filename: Value::str(filename),
            filename2: Value::None,
            args: Vec::new(),
        };
        // ⚠ 引数は整数と文字列だけなので `str` / `repr` は失敗しない（利用者の `__str__` を呼ばない）。
        let shown = self.os_error_message(&os).unwrap_or_else(|e| e);
        let msg = format!("{class}: {shown}");
        self.error_with_args(msg, vec![os.errno, os.strerror, os.filename])
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
