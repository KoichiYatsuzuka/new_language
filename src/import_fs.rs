//! import_fs.rs — import の処理が**外界に触る所**の唯一の窓口
//! （ファイル・ディレクトリ・環境変数・カレントディレクトリ・Python の場所）。
//!
//! # なぜ要るか（`implementation_plans/editor_import_resolution_plan.md` 2-1）
//!
//! VS Code 拡張（`crates/arrow-frontend` の wasm）でも CLI と**同じ import の処理**
//! （`parser/imports/`・[`crate::module_path`]・`ar_config`・.NET のメタデータ…）を使うため。
//! wasm はファイルも環境変数も持たないので、拡張ではここだけをホスト（拡張の TypeScript 側）に
//! 頼む実装に差し替える。import の探索・読み込みの**規則**は呼び出し側に 1 つだけあり、
//! ここは「読む」「在るか見る」だけを受け持つ（判断をしない）。
//!
//! ⚠⚠ import の処理は `std::fs` / `std::env` を**直接呼ばない**こと。ここを通さない呼び出しは
//!    拡張では動かない（CLI だけで動いて拡張では型が付かない、という食い違いに戻る）。

use std::path::{Path, PathBuf};

/// `p` が在るか（ファイルでもディレクトリでも）。
pub fn exists(p: &Path) -> bool {
    p.exists()
}

/// `p` がディレクトリか。
pub fn is_dir(p: &Path) -> bool {
    p.is_dir()
}

/// `p` の中身（バイト列）。
pub fn read(p: &Path) -> std::io::Result<Vec<u8>> {
    std::fs::read(p)
}

/// `p` の中身（UTF-8 のテキスト）。
pub fn read_to_string(p: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(p)
}

/// `p` へ書く（import の副産物を書き出す所だけが使う・拡張では何もしない予定）。
pub fn write(p: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::write(p, contents)
}

/// 環境変数 `name` の値。無ければ `None`。
pub fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// カレントディレクトリ。取れなければ `None`。
pub fn current_dir() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

/// Python の**標準ライブラリと site-packages の場所**（この順）。Python が見つからなければ空。
///
/// Python プロセスを 1 回だけ起動して `sysconfig` に尋ねる（`OnceLock` で保持する）。
/// ⚠ import の処理で外部プロセスを起動するのはここだけ。拡張ではホストが一度だけ調べて渡す。
pub fn python_lib_dirs() -> &'static [PathBuf] {
    use std::sync::OnceLock;
    static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
    DIRS.get_or_init(|| {
        let script = concat!(
            "import sysconfig; ",
            "paths = [sysconfig.get_path('stdlib'), sysconfig.get_path('purelib')]; ",
            "print('\\n'.join(p for p in paths if p))"
        );
        #[cfg(windows)]
        let candidates = ["py", "python", "python3"];
        #[cfg(not(windows))]
        let candidates = ["python3", "python"];
        for exe in candidates {
            let Ok(out) = std::process::Command::new(exe).args(["-c", script]).output() else {
                continue;
            };
            if !out.status.success() {
                continue;
            }
            if let Ok(s) = String::from_utf8(out.stdout) {
                let dirs: Vec<PathBuf> = s
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(PathBuf::from)
                    .collect();
                if !dirs.is_empty() {
                    return dirs;
                }
            }
        }
        vec![]
    })
}
