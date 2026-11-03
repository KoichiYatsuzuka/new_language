// py_stubs.rs — `import[py-int]` 用に**バイナリへ埋め込む**型スタブ（BYTECODE_VM_PLAN #19）。
//
// ★ `time` / `math` は CPython の**C 組み込みモジュール**で `.py` が存在しない。
//   そのため `Parser::python_search_dirs_from` が stdlib まで辿っても必ず空振りし、
//   `load_python_interface_module` は空の body（＝型検査を丸ごと放棄）を返していた。
//   ⇒ `let s: str = math.sqrt(2.0)` が**黙って通って**いた。
//
// ⚠⚠ **検索ディレクトリを増やす案は採らなかった**（群6 S2）:
//   - `src/` に std の current_exe() の使用は 0 箇所。`cargo run`（`target/debug/`）と配布
//     レイアウトで相対位置が変わり、「開発中だけ効く」を作り込む。
//   - `python_search_dirs_from` に 1 つ足すと**モジュール解決 1 回あたり `exists()` が
//     4 回増える**。#69 が「`exists()` の syscall 連打が `interp_init` の 48〜53%」と
//     実測して遅延化した場所で、同じ轍になる。
//   ⇒ `include_str!` で埋め込む。syscall ゼロ・パス依存ゼロ。
//
// ⚠ **利用者のファイルが常に勝つ**。この表を引くのは、検索ディレクトリを全部見て
//   1 つも見つからなかったときだけ（`load_python_interface_module` の末尾）。
//   自分で `time.pyi` を置けば上書きできる。
//
// ⚠ 置き場所が `src/python_converter/` ではなく `src/` 直下なのは、
//   `crates/arrow-frontend`（VS Code 拡張の wasm）が `src/parser` を取り込むため。
//   変換器側に置くと拡張のビルドだけが壊れる（項目 7 で `PY_KWARGS_PARAM` が
//   同じ理由で `src/ast.rs` へ行ったのと同じ判断）。
//
// ★ スタブは 2 種類ある（pandas_import_plan.md の 1-4）:
//   - 手で書いたもの（`stubs/*.pyi`・`BUILTIN_STUBS`）: 戻り値の型まで書いてある。`time` / `math` だけ。
//   - CPython の内省で作ったもの（`stubs/generated/*.pyi`・`py_stubs_generated.rs` の `GENERATED_STUBS`）: `sys` / `itertools` /
//     `_collections` など、`.py` の無い built-in と標準ライブラリの拡張モジュール（`select` など）。引数の名前と数は
//     CPython の __text_signature__ から、大域変数の型は作ったときの値の型から付け、それ以外は `Any`。
//     `scripts/gen_builtin_py_stubs.ps1` で作り直す（手で書き換えない）。
//   どちらにも無いモジュールは、型の出所が無いので `import[py-int]` が構文解析の誤りになる（`.pyi` を置くよう促す）。
//   ⚠ 内省で作ったものは作った CPython（版と OS）のもの。Linux だけのモジュール（posix / fcntl など）は無い。

include!("py_stubs_generated.rs");

/// 手で書いた同梱スタブの表（モジュール名 → `.pyi` の中身）。内省で作った表より先に引く。
const BUILTIN_STUBS: &[(&str, &str)] = &[
    ("time", include_str!("../stubs/time.pyi")),
    ("math", include_str!("../stubs/math.pyi")),
];

/// モジュールパスに対応する同梱スタブを返す（手で書いたもの、なければ内省で作ったもの）。
///
/// ⚠ トップレベルの 1 要素モジュールだけを対象にする。`os.path` のようなサブモジュールは持っていない
/// （C で書かれたモジュールはどれも 1 要素）。
pub fn builtin_stub(module: &[String]) -> Option<&'static str> {
    if module.len() != 1 {
        return None;
    }
    let name = module[0].as_str();
    BUILTIN_STUBS
        .iter()
        .chain(GENERATED_STUBS.iter())
        .find(|(m, _)| *m == name)
        .map(|(_, src)| *src)
}

/// 同梱スタブを持つモジュール名の一覧（テスト・診断用）。
#[cfg(test)]
pub fn builtin_stub_names() -> Vec<&'static str> {
    BUILTIN_STUBS.iter().chain(GENERATED_STUBS.iter()).map(|(m, _)| *m).collect()
}
