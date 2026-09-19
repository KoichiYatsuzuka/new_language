// py_stubs.rs — `import[py-int]` 用に**バイナリへ埋め込む**型スタブ（BYTECODE_VM_PLAN #19）。
//
// ★ `time` / `math` は CPython の**C 組み込みモジュール**で `.py` が存在しない。
//   そのため `Parser::python_search_dirs()` が stdlib まで辿っても必ず空振りし、
//   `load_python_interface_module` は空の body（＝型検査を丸ごと放棄）を返していた。
//   ⇒ `let s: str = math.sqrt(2.0)` が**黙って通って**いた。
//
// ⚠⚠ **検索ディレクトリを増やす案は採らなかった**（群6 S2）:
//   - `src/` に std の current_exe() の使用は 0 箇所。`cargo run`（`target/debug/`）と配布
//     レイアウトで相対位置が変わり、「開発中だけ効く」を作り込む。
//   - `python_search_dirs()` に 1 つ足すと**モジュール解決 1 回あたり `exists()` が
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

/// 同梱スタブの表（モジュール名 → `.pyi` の中身）。
const BUILTIN_STUBS: &[(&str, &str)] = &[
    ("time", include_str!("../stubs/time.pyi")),
    ("math", include_str!("../stubs/math.pyi")),
];

/// モジュールパスに対応する同梱スタブを返す。
///
/// ⚠ トップレベルの 1 要素モジュール（`time` / `math`）だけを対象にする。
/// `os.path` のようなサブモジュールは持っていない。
pub fn builtin_stub(module: &[String]) -> Option<&'static str> {
    if module.len() != 1 {
        return None;
    }
    let name = module[0].as_str();
    BUILTIN_STUBS
        .iter()
        .find(|(m, _)| *m == name)
        .map(|(_, src)| *src)
}

/// 同梱スタブを持つモジュール名の一覧（テスト・診断用）。
#[cfg(test)]
pub fn builtin_stub_names() -> Vec<&'static str> {
    BUILTIN_STUBS.iter().map(|(m, _)| *m).collect()
}
