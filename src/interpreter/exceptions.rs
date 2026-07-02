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
        const CATCHABLE: &[&str] = &[
            "ZeroDivisionError",
            "NotImplementedError",
            "AttributeError",
            "ArithmeticError",
            "AssertionError",
            "OverflowError",
            "AccessError",
            "RuntimeError",
            "ValueError",
            "TypeError",
            "NameError",
            "IndexError",
            "KeyError",
            "IOError",
            "OSError",
            "StopIteration",
            "Exception",
        ];

        let class_name = CATCHABLE.iter().find(|&&cn| {
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

        // フィールドレイアウト: message=0, code_context=1, file=2, line=3, col=4
        // (make_error_class の field_index と対応)
        let fields = vec![
            Some((Value::Str(message), false)),       // 0: message
            Some((Value::Str(String::new()), false)),  // 1: code_context / Error::code_context
            Some((Value::Str(String::new()), false)),  // 2: file / Error::file
            Some((Value::Int(0), false)),              // 3: line / Error::line
            Some((Value::Int(0), false)),              // 4: col / Error::col
        ];

        let inst = Value::Instance(Rc::new(RefCell::new(InstanceData {
            class_id: cls.class_id,
            flags: crate::interpreter::value::INST_IS_EXCEPTION,
            class: cls,
            fields,
        })));

        Some(RaisedError {
            exception: inst,
            frames: vec![],
        })
    }

    /// 例外インスタンスのクラスが `except` 節の型名にマッチするか判定する。
    ///
    /// マッチ条件: クラス名が `type_name` と一致するか、または `bases`（基底クラス・trait）に含まれる場合。
    ///
    /// - `inst_class`: 例外インスタンスのクラス定義
    /// - `type_name`: `except` 節で指定された型名
    ///
    /// 戻り値: `true` — マッチあり（例外がこの handler で捕捉される）
    pub(super) fn exc_matches(inst_class: &Rc<ClassValue>, type_name: &str) -> bool {
        inst_class.name == type_name || inst_class.bases.contains(&type_name.to_string())
    }
}
