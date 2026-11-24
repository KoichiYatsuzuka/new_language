use super::CompileMode::*;
use super::CompileMode;

/// **#52 で 3 つの bool を `CompileMode` へ畳んだときの対応表**を固定する。
///
/// ⚠ #52 以前は各入口の構造体リテラルに `module_mode` / `name_lookup` / `debug_mode` が
/// 並んでいたので、値は**読めば分かった**。畳んだ後は `matches!` のアームを 1 つ書き換えると
/// **静かに全入口の挙動が変わる**。ここが唯一の防波堤。
///
/// 期待値は #51 時点の実装から写したもの（左から module_mode / name_lookup / debug_mode）。
#[test]
pub(super) fn mode_predicates_match_the_pre_52_flags() {
    let expected: &[(CompileMode, bool, bool, bool)] = &[
        //                     is_module_body / uses_name_lookup / is_debug_repl
        (Function, false, false, false),
        (Toplevel, false, false, false),
        (Module, true, false, false),
        (DefinitionExpr, false, true, false),
        (AsyncBody, false, false, false),
        (DebugRepl, false, true, true),
    ];
    for &(m, module_body, name_lookup, debug_repl) in expected {
        assert_eq!(m.is_module_body(), module_body, "is_module_body for {m:?}");
        assert_eq!(m.uses_name_lookup(), name_lookup, "uses_name_lookup for {m:?}");
        assert_eq!(m.is_debug_repl(), debug_repl, "is_debug_repl for {m:?}");
    }
}

/// `uses_name_lookup` は畳む前の **`debug_mode || name_lookup`** と一致すること。
/// ⚠ 畳めた根拠は「`DebugRepl` は `name_lookup` も真だった」の 1 点だけなので、
/// `DebugRepl` を名前引きでなくするなら**この畳み込みも解く**必要がある。
#[test]
pub(super) fn debug_repl_implies_name_lookup() {
    assert!(DebugRepl.uses_name_lookup());
}
