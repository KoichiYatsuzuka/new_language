// ar_config.rs — `ar_config.json` の読み取り（共有・#72）。
//
// ## ⚠⚠ このファイルが持つのは「**1 個のファイルから何を読むか**」だけ
//
// 「**どのファイルを読むか**」（探索方針）は**呼び出し側それぞれの契約**で、
// #72 で調べたところ **5 つの読み手が 4 通りの方針**を持っており、
// **どれにも理由がある**（畳むと挙動が変わる）:
//
// | 読み手 | セクション | 探索方針 | 理由 |
// |---|---|---|---|
// | `Interpreter::python_search_dirs()`（`load_python_search_paths`） | `python.search_paths` | `source_dir` から**祖先を root まで**遡り、**最初の 1 個で打ち切り** | 上位の設定が下位を上書きしない、という既存の挙動（#61） |
// | `Parser::python_search_dirs()` | `python.search_paths` | **`source_dir` と `root_dir` の 2 箇所だけ** | `root_dir` は**エントリのディレクトリ**でサブパーサにも引き継ぐ、というパーサ側の契約 |
// | `cpp_bridge::config::load_cpp_config` | `cpp.*` | 祖先を root まで ＋ cwd を**全部レイヤーマージ**（遠い方から適用） | **打ち切りだと中間の部分的な設定がルートの cpp 設定を丸ごと隠す**（実バグとして修正済み） |
// | `exec::find_js_config` | `javascript.*` | `python_search_dirs` → cwd の順に**最初に見つかったもの** | ブリッジは 1 つだけ要るため |
// | `Parser::load_cs_lib_paths` | `csharp.lib_paths` | `source_dir` から**祖先を root まで**・最初の 1 個 | `python` 側と同じ（#73 で読み取りを共有化） |
//
// ⚠ **`python` の 2 つは方針が食い違ったままである**（祖先全走査 ↔ 2 箇所）。
// つまり中間の祖先にある設定は `import[py-int]` からは見えて `import[py]` からは見えない。
// #72 では**そこは変えていない**（どちらの契約にも理由があり、揃えると解決順が動く）。
// ⇒ 揃えるなら独立タスクで、**先に「どちらへ揃えるか」を決めること**。
//
// ## #72 が実際に畳んだもの
//
// `python.search_paths` の**読み取り実装が 2 つ**あった:
// - ここ（serde）
// - `parser/imports/mod.rs::parse_python_search_paths`（**手書きの文字列走査**・削除済み）
//
// #73 で `csharp.lib_paths` の手書き走査（`parse_cs_lib_paths`・削除済み）も同じ形で畳んだ。
// **そちらはセクション名で絞ってすらいなかった**ので誤りが 1 つ多い（11 ケース中 6 件相違）。
//
// 差分計測（14 ケース）で **5 件食い違い**、うち **3 件は手書き側の誤り**だった:
//
// | ケース | 手書き | serde |
// |---|---|---|
// | `{"python":{},"rust":{"search_paths":["x"]}}` | `x` を拾う（**誤り**: `python` の外） | 空 |
// | `{"python":{"search_paths":[1,"a"]}}` | `1` をパス扱い（**誤り**） | `a` のみ |
// | `{"python":{"search_paths":["a,b"]}}` | `a` と `b` に割る（**誤り**: 区切りと誤認） | `a,b` |
// | `{"python":{"search_paths":["","a"]}}` | 空要素を捨てる（**こちらが妥当**） | ← 揃えた（下記） |
// | 壊れた JSON | 拾えるだけ拾う | 空 |

use std::path::{Path, PathBuf};

/// `source_dir` から**祖先へ遡って** `ar_config.json` を探し、最初に見つけたものの
/// `python.search_paths` を（相対なら設定ファイルのある場所を基準に）絶対パス化して返す（#61）。
///
/// 消費者は `Interpreter::python_search_dirs()`（`import[py-int]` と
/// cs-dll / cs-proc / js-proc のブリッジ探索）。
///
/// ⚠ **見つかった時点で打ち切る**（読めなくても・壊れていても遡らない）。
/// 上位の設定が下位を上書きしない、という既存の挙動をそのまま保つため。
/// ⚠ **見つからない場合はドライブ root まで遡る**（打ち切りが無い）。
/// これが `interp_init` の支配項だったので、**呼び出しは遅延させてある**（#69）。
pub(crate) fn load_python_search_paths(source_dir: &Path) -> Vec<PathBuf> {
    let mut walk: Option<&Path> = Some(source_dir);
    while let Some(d) = walk {
        let cfg_path = d.join("ar_config.json");
        if cfg_path.exists() {
            return read_python_search_paths(&cfg_path, d);
        }
        walk = d.parent();
    }
    Vec::new()
}

/// `ar_config.json` **1 個**から `python.search_paths` を読む（#61・#72 で共有化）。
///
/// 読めない・壊れている・キーが無い場合は**空**（エラーにはしない＝従来どおり）。
/// 相対パスは `base`（＝設定ファイルのあるディレクトリ）基準で絶対化する。
///
/// ⚠ **空文字列の要素は捨てる**。捨てないと `base.join("")` ＝ `base` 自身が検索パスに入り、
/// 「設定に空文字を書いたら設定ファイルの場所が検索対象になる」という意味不明な挙動になる
/// （#72 以前の手書き実装はこちらの挙動で、そちらが妥当だったので揃えた）。
pub(crate) fn read_python_search_paths(cfg_path: &Path, base: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(cfg_path) else {
        return Vec::new();
    };
    read_python_search_paths_from_str(&text, base)
}

/// [`read_python_search_paths`] の中身（テストから直接 JSON を渡せるように分けてある・#72）。
pub(crate) fn read_python_search_paths_from_str(text: &str, base: &Path) -> Vec<PathBuf> {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(paths) = root
        .get("python")
        .and_then(|p| p.get("search_paths"))
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    resolve_path_array(paths, base)
}

/// `ar_config.json` のテキストから `csharp.lib_paths` を読む（#73 で共有化）。
///
/// `import[cs-dll]` / `import[cs-proc]` が既定の候補パスで DLL を見つけられなかったときの
/// **追加検索ディレクトリ**。空なら `None`（呼び出し側が「設定なし」と同じ扱いをするため）。
///
/// ⚠⚠ **セクション（`csharp`）で必ず絞ること。** #73 以前の手書き走査は
/// `"lib_paths"` を **JSON 全文から探して**いたので、`cpp.lib_paths` のような
/// 別セクションの値や、トップレベルの `lib_paths` を**C# の DLL 探索に使ってしまう**状態だった
/// （差分計測 11 ケース中 6 件相違・うち 5 件が手書き側の誤り）。
///
/// ⚠ `python` 側と違って**パスを取る版は用意しない** — 呼び出し側
/// （`Parser::load_cs_lib_paths`）は「存在するが読めない設定ファイル」のとき
/// **さらに上へ遡る**という独自の方針を持っており、読み込みを自分で行う必要がある。
pub(crate) fn read_cs_lib_paths_from_str(text: &str, base: &Path) -> Option<Vec<PathBuf>> {
    let root: serde_json::Value = serde_json::from_str(text).ok()?;
    let arr = root.get("csharp")?.get("lib_paths")?.as_array()?;
    let v = resolve_path_array(arr, base);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// JSON の文字列配列を `base` 基準の絶対パス列にする（#73 で `python` 版と共有）。
///
/// ⚠ **文字列でない要素と空文字列は捨てる。** 空文字を残すと `base.join("")` ＝ `base` 自身が
/// 検索パスに入り、「設定に空文字を書いたら設定ファイルの場所が検索対象になる」ことになる。
fn resolve_path_array(arr: &[serde_json::Value], base: &Path) -> Vec<PathBuf> {
    arr.iter()
        .filter_map(|p| p.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| {
            let pb = PathBuf::from(s);
            if pb.is_absolute() {
                pb
            } else {
                base.join(pb)
            }
        })
        .collect()
}


#[cfg(test)]
mod tests {
    use super::read_python_search_paths_from_str as read;
    use std::path::{Path, PathBuf};

    fn v(base: &str, xs: &[&str]) -> Vec<PathBuf> {
        xs.iter().map(|x| Path::new(base).join(x)).collect()
    }

    /// ⚠⚠ **#72 以前の手書き走査が間違えていた形**を固定する。
    /// 削除した `parse_python_search_paths` はこの 3 つを**全部取り違えていた**
    /// （差分計測で判明）。同じ実装をまた書かないための番人。
    #[test]
    fn serde_reader_fixes_hand_rolled_scanner_bugs() {
        let b = Path::new("/BASE");
        // ① `python` の**外**にある `search_paths` を拾わない（手書きは拾っていた）
        assert!(read(r#"{"python":{},"rust":{"search_paths":["x"]}}"#, b).is_empty());
        // ② 文字列でない要素を捨てる（手書きは `1` をパスにしていた）
        assert_eq!(read(r#"{"python":{"search_paths":[1,"a"]}}"#, b), v("/BASE", &["a"]));
        // ③ パス中のカンマで割らない（手書きは配列区切りと誤認していた）
        assert_eq!(read(r#"{"python":{"search_paths":["a,b"]}}"#, b), v("/BASE", &["a,b"]));
        // ④ 壊れた JSON は空（手書きは拾えるだけ拾っていた）
        assert!(read(r#"{"python":{"search_paths":["a"]"#, b).is_empty());
    }

    /// 空文字列は捨てる（`base` 自身が検索パスに入るのを防ぐ・#72）。
    #[test]
    fn empty_string_entries_are_dropped() {
        let b = Path::new("/BASE");
        assert_eq!(read(r#"{"python":{"search_paths":["","a"]}}"#, b), v("/BASE", &["a"]));
    }

    /// 絶対パスはそのまま、相対パスは設定ファイルの場所基準。
    ///
    /// ⚠ 「絶対パス」の書き方は OS で違う — Windows では `/abs` は
    /// **ドライブが無いので絶対ではない**（`is_absolute()` が偽）。
    #[test]
    fn relative_paths_resolve_against_the_config_dir() {
        let b = Path::new("/BASE");
        let abs = if cfg!(windows) { "C:/abs" } else { "/abs" };
        let json = format!(r#"{{"python":{{"search_paths":["{abs}","rel"]}}}}"#);
        let got = read(&json, b);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0], PathBuf::from(abs), "absolute path must pass through");
        assert_eq!(got[1], Path::new("/BASE").join("rel"));
    }

    /// `python` セクションが無い / `search_paths` が無い → 空（エラーにしない）。
    #[test]
    fn missing_sections_yield_empty() {
        let b = Path::new("/BASE");
        assert!(read(r#"{"rust":{"crates_path":"x"}}"#, b).is_empty());
        assert!(read(r#"{"python":{"other":1}}"#, b).is_empty());
    }

    /// ⚠⚠ **#73 以前の手書き走査（`parse_cs_lib_paths`）が間違えていた形**を固定する。
    /// そちらは `"lib_paths"` を **JSON 全文から探して**いたので、
    /// **セクション違いの値を C# の DLL 探索に使って**いた。
    #[test]
    fn cs_reader_requires_the_csharp_section() {
        use super::read_cs_lib_paths_from_str as read_cs;
        let b = Path::new("/BASE");
        // ① トップレベルの `lib_paths` は拾わない（手書きは拾っていた）
        assert!(read_cs(r#"{"lib_paths":["top"]}"#, b).is_none());
        // ② **別セクション**の `lib_paths` は拾わない（手書きは `cpp` 側を返していた）
        assert_eq!(
            read_cs(r#"{"cpp":{"lib_paths":["cpp1"]},"csharp":{"lib_paths":["cs1"]}}"#, b),
            Some(v("/BASE", &["cs1"]))
        );
        assert!(read_cs(r#"{"cpp":{"lib_paths":["cpp1"]}}"#, b).is_none());
        // ③ 文字列でない要素を捨てる／カンマで割らない／壊れた JSON は None
        assert_eq!(read_cs(r#"{"csharp":{"lib_paths":[1,"a"]}}"#, b), Some(v("/BASE", &["a"])));
        assert_eq!(read_cs(r#"{"csharp":{"lib_paths":["a,b"]}}"#, b), Some(v("/BASE", &["a,b"])));
        assert!(read_cs(r#"{"csharp":{"lib_paths":["a"]"#, b).is_none());
    }

    /// 空配列・空文字列だけ → `None`（「設定なし」と同じ扱い・#73）。
    #[test]
    fn cs_empty_yields_none() {
        use super::read_cs_lib_paths_from_str as read_cs;
        let b = Path::new("/BASE");
        assert!(read_cs(r#"{"csharp":{"lib_paths":[]}}"#, b).is_none());
        assert!(read_cs(r#"{"csharp":{"lib_paths":[""]}}"#, b).is_none());
    }
}
