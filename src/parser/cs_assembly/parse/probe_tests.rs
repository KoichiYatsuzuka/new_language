use super::*;
use crate::ast::Expr;

/// `examples/interop/cs_probe/cs_probe.dll`（build_cs_probe.ps1・nullable 有効）の型スタブ。
/// キーは `クラス.メソッド`、値は `(引数の型, 戻り値の型, 未対応の理由)`。
fn probe_dll() -> std::path::PathBuf {
    // ⚠ 拡張のクレート（`crates/arrow-frontend`）も同じファイルを取り込んでテストするので、リポジトリの直下を
    //   クレートの場所から上へ探す（`compare_wasm_frontend.ps1` の前段で走る）。
    let rel = Path::new("examples/interop/cs_probe/cs_probe.dll");
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .map(|d| d.join(rel))
        .find(|p| p.exists())
        .expect("examples/interop/cs_probe/cs_probe.dll (build_cs_probe.ps1)")
}

fn probe_stubs() -> HashMap<String, (Vec<String>, String, Option<String>)> {
    probe_stubs_for(CsBridge::Dll)
}

fn probe_stubs_for(bridge: CsBridge) -> HashMap<String, (Vec<String>, String, Option<String>)> {
    let dll = probe_dll();
    let mut out = HashMap::new();
    for st in load_cs_assembly(&dll, bridge).expect("cs_probe.dll") {
        let Stmt::ClassDef { name: cls, body, .. } = st else { continue };
        for m in body {
            let Stmt::FnDef { name, params, return_type, body, .. } = m else { continue };
            let ps = params.iter().filter(|p| p.name != "self").map(|p| p.type_ann.clone().unwrap_or_default()).collect();
            let reason = match body.first() {
                Some(Stmt::Expr(Expr::Str(s))) => s.strip_prefix(crate::cpp_header::FFI_UNSUPPORTED_MARK).map(str::to_string),
                _ => None,
            };
            out.insert(format!("{cls}.{name}"), (ps, return_type.unwrap_or_default(), reason));
        }
    }
    out
}

/// `string?` / `Person?` / `long?` は `Option[T]`、注釈の無い参照型は `T` のまま（2-1）。
#[test]
fn nullable_annotations_become_option() {
    let st = probe_stubs();
    let sig = |k: &str| {
        let (ps, r, _) = st.get(k).unwrap_or_else(|| panic!("{k} missing"));
        (ps.iter().map(String::as_str).collect::<Vec<_>>().join(", "), r.clone())
    };
    assert_eq!(sig("Probe.greet"), (String::new(), "str".to_string()));
    assert_eq!(sig("Probe.maybe"), ("bool".to_string(), "Option[str]".to_string()));
    assert_eq!(sig("Probe.nothing").1, "Option[str]");
    assert_eq!(sig("Probe.maybe_num").1, "Option[int]");
    assert_eq!(sig("Probe.find"), ("str".to_string(), "Option[Person]".to_string()));
    assert_eq!(sig("Probe.describe_opt"), ("Option[Person]".to_string(), "str".to_string()));
    assert_eq!(sig("Probe.add_opt"), ("int, Option[int]".to_string(), "int".to_string()));
    assert_eq!(sig("Probe.make_person").1, "Person");
    assert_eq!(sig("Person.best_friend").1, "Option[Person]");
    // 値型の `char` は nullable の文脈に関わらず `str` のまま
    assert_eq!(sig("Probe.initial").1, "str");
    assert_eq!(sig("Probe.any_obj").1, "Any");
}

/// cs-proc はコレクションと `bytes` をコピーで運ぶ（5-2）。中身が受け渡せない型なら未対応のまま。
#[test]
fn cs_proc_carries_collections() {
    let st = probe_stubs_for(CsBridge::Proc);
    let why = |k: &str| st.get(k).unwrap_or_else(|| panic!("{k} missing")).2.clone();
    for k in ["Probe.words", "Probe.words_list", "Probe.counts", "Probe.ints", "Probe.sum", "Probe.join", "Probe.pair",
              "Person.nicknames"] {
        assert_eq!(why(k), None, "{k}");
    }
    assert_eq!(st.get("Probe.counts").map(|(_, r, _)| r.as_str()), Some("dict[str,int]"));
    assert!(why("Probe.apply").is_some_and(|w| w.contains("delegate")));
    assert!(why("Probe.first").is_some(), "a generic method stays unsupported");
    assert!(why("Probe.builder").is_some(), "another assembly's type stays unsupported");
}

/// 公開のコンストラクタは `__init__` のオーバーロードになる（5-1・実行時はこの型で引数を並べる）。
#[test]
fn constructors_become_init_overloads() {
    let st = probe_stubs();
    let (ps, ret, why) = st.get("Person.__init__").expect("Person.__init__");
    assert_eq!((ps.join(", "), ret.as_str(), why), ("str, int".to_string(), "None", &None));
    assert_eq!(st.get("Probe.__init__").map(|(ps, _, _)| ps.len()), Some(0), "the implicit default constructor");
}

/// `--gen-cs-exports`（5-1）: 型スタブと同じ名前・型の export を作り、作らないものは理由を残す。
#[cfg(not(feature = "editor"))]
#[test]
fn generates_exports_for_the_probe() {
    let text = generate_cs_exports_text(&probe_dll()).expect("exports");
    for entry in ["Person_new_2", "Person_inst_hello", "Person_inst_older", "Probe_greet", "Probe_echo", "Probe_maybe",
                  "Probe_initial", "Probe_twice", "Probe_low_byte", "Probe_huge_ulong", "Probe_add_opt", "Probe_describe_opt",
                  "arrow_bridge_release", "arrow_bridge_free_str", "arrow_bridge_take_error"] {
        assert!(text.contains(&format!("EntryPoint = \"{entry}\"")), "{entry}");
    }
    // 整数の幅と `ulong` の戻り値を確かめる・nullable のクラスの引数は 0 を null にする・`long?` は値型の nullable
    assert!(text.contains("return ArrowAbi.FromUInt64(r, what);"), "ulong return");
    assert!(text.contains("a0 == 0 ? null : ArrowObjects.Get<global::Person>(a0, 1, what)"), "Person? argument");
    assert!(text.contains("(long?)(a1)"), "long? argument");
    assert!(text.contains("return ArrowObjects.Store(new global::Person(ArrowAbi.Str(a0, a0n)!, a1));"), "constructor");
    assert!(text.contains("catch (Exception e) { ArrowAbi.SetError(e);"), "exceptions are caught");
    // 5-5: 配列・list・bytes（JSON / ArPtrProtocol）
    assert!(text.contains("ArrowJson.WriteList<string>(r, (w, x) => { ArrowJson.WStr(w, x); }, outPtr, outLen)"), "string[] return");
    assert!(text.contains("ArrowAbi.WriteArray<long>(r, \"int64\", outArr)"), "long[] return");
    assert!(text.contains("ArrowAbi.Array<long>(a0, \"int64\", 1, what)!"), "long[] argument");
    // 作らないものは理由つき
    assert!(text.contains("// skipped Probe.counts: it returns `dict[str,int]`"), "dict (cs-dll)");
    assert!(text.contains("// skipped Probe.any_str: it returns `object`"), "object return");
    assert!(text.contains("// skipped Probe.first: "), "generic method");
}

/// `ref` / `out` の引数を持つメソッドは、どちらのブリッジでも未対応（7-16。書き戻さないので）。
#[test]
fn byref_methods_are_marked() {
    for bridge in [CsBridge::Dll, CsBridge::Proc] {
        let st = probe_stubs_for(bridge);
        let why = |k: &str| st.get(k).unwrap_or_else(|| panic!("{k} missing")).2.clone();
        assert_eq!(
            why("Probe.try_parse").as_deref(),
            Some("parameter `value` is `ref` / `out`, which Arrow does not write back")
        );
        assert!(why("Probe.bump").is_some_and(|w| w.contains("`ref` / `out`")));
        assert_eq!(why("Probe.greet"), None);
    }
}

/// cs-dll: dict・set・タプル・デリゲート・型変数・他のアセンブリの型を使うメソッドだけに印が付く（2-1・5-5）。
/// 配列・list・bytes は 5-5 から受け渡せる（数値の配列は `Ptr[T]`）。
#[test]
fn unsupported_methods_are_marked() {
    let st = probe_stubs();
    let why = |k: &str| st.get(k).unwrap_or_else(|| panic!("{k} missing")).2.clone();
    assert!(why("Probe.counts").is_some_and(|w| w.starts_with("it returns `") && w.contains("a C# collection")));
    for k in ["Probe.words", "Probe.words_list", "Probe.ints", "Probe.sum", "Probe.join", "Probe.raw", "Probe.halves"] {
        assert_eq!(why(k), None, "{k}");
    }
    let ty = |k: &str| st.get(k).map(|(ps, r, _)| (ps.join(", "), r.clone())).unwrap_or_default();
    assert_eq!(ty("Probe.ints").1, "Ptr[int64]");
    assert_eq!(ty("Probe.sum").0, "Union[Ptr[int64], list[int]]");
    assert_eq!(ty("Probe.words").1, "list[str]");
    assert!(why("Probe.apply").is_some_and(|w| w.contains("delegate")));
    assert!(why("Probe.pair").is_some_and(|w| w.contains("tuple")));
    assert!(why("Probe.first").is_some_and(|w| w == "it returns `T`, a generic type parameter, which cannot cross the bridge"));
    assert!(why("Probe.builder").is_some_and(|w| w.contains("System.Text.StringBuilder")));
    for k in ["Probe.greet", "Probe.make_person", "Probe.find", "Probe.describe_opt", "Probe.any_obj",
              "Probe.initial", "Probe.add_opt", "Person.older", "Person.best_friend"] {
        assert_eq!(why(k), None, "{k}");
    }
}
