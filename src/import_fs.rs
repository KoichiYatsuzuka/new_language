//! import_fs.rs — import の処理が**外界に触る所**の唯一の窓口
//! （ファイル・ディレクトリ・環境変数・カレントディレクトリ・Python の場所）。
//!
//! # なぜ要るか（`implementation_plans/editor_import_resolution_plan.md` 2-1）
//!
//! VS Code 拡張（`crates/arrow-frontend` の wasm）でも CLI と**同じ import の処理**
//! （`parser/imports/`・[`crate::module_path`]・`ar_config`・.NET のメタデータ…）を使うため。
//! wasm はファイルも環境変数も持たないので、拡張ではここだけをホスト（拡張の TypeScript 側・
//! `vscode-extension/src/wasm_host.ts`）に頼む実装に差し替える（3-1）。import の探索・読み込みの
//! **規則**は呼び出し側に 1 つだけあり、ここは「読む」「在るか見る」だけを受け持つ（判断をしない）。
//!
//! ⚠⚠ import の処理は `std::fs` / `std::env` を**直接呼ばない**こと。ここを通さない呼び出しは
//!    拡張では動かない（CLI だけで動いて拡張では型が付かない、という食い違いに戻る）。
//!
//! # wasm 版のパスの形
//!
//! wasm の `std::path` は Unix の規則なので、Windows のパスは `/D:/a/b.ar` の形で受け渡す
//! （ホストが境界で変換する・`wasm_host.ts` の `toWasmPath`）。ここで受け取るパスも返すパスも
//! この形で、Rust 側は区別しなくてよい。

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use host::*;

/// CLI（と、ネイティブでビルドした `arrow-frontend` のテスト）: `std::fs` / `std::env` をそのまま使う。
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};

    /// `p` が在るか（ファイルでもディレクトリでも）。
    pub fn exists(p: &Path) -> bool {
        p.exists()
    }

    /// `p` がディレクトリか。
    pub fn is_dir(p: &Path) -> bool {
        p.is_dir()
    }

    /// `p` を絶対パスにし、シンボリックリンクなどを解いた形（同じファイルを 1 回だけ読むための鍵）。
    /// 取れなければ `None`（呼び出し側は元のパスを使う）。
    pub fn canonicalize(p: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(p).ok()
    }

    /// ディレクトリ `p` の中の項目のパス（順不同）。
    pub fn read_dir(p: &Path) -> std::io::Result<Vec<PathBuf>> {
        Ok(std::fs::read_dir(p)?.flatten().map(|e| e.path()).collect())
    }

    /// `p` の中身（バイト列）。
    pub fn read(p: &Path) -> std::io::Result<Vec<u8>> {
        std::fs::read(p)
    }

    /// `p` の中身（UTF-8 のテキスト）。
    pub fn read_to_string(p: &Path) -> std::io::Result<String> {
        std::fs::read_to_string(p)
    }

    /// `p` へ書く（import の副産物を書き出す所だけが使う・拡張では何もしない）。
    pub fn write(p: &Path, contents: &str) -> std::io::Result<()> {
        std::fs::write(p, contents)
    }

    /// 環境変数 `name` を**1 つのパス**として読む。無ければ `None`。
    pub fn env_path(name: &str) -> Option<PathBuf> {
        std::env::var(name).ok().map(PathBuf::from)
    }

    /// 環境変数 `name` を**パスの並び**（`PATH` と同じ区切り）として読む。無ければ空。
    pub fn env_paths(name: &str) -> Vec<PathBuf> {
        match std::env::var(name) {
            Ok(v) => std::env::split_paths(&v).collect(),
            Err(_) => Vec::new(),
        }
    }

    /// カレントディレクトリ。取れなければ `None`。
    pub fn current_dir() -> Option<PathBuf> {
        std::env::current_dir().ok()
    }

    /// Python の**標準ライブラリと site-packages の場所**（この順）。Python が見つからなければ空。
    ///
    /// Python プロセスを 1 回だけ起動して `sysconfig` に尋ねる（`OnceLock` で保持する）。
    /// ⚠ import の処理で外部プロセスを起動するのはここだけ。拡張ではホストが一度だけ調べて渡す
    ///   （`wasm_host.ts` の `queryPythonDirs`・**同じ問い合わせ**を保つこと）。
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
}

/// 拡張（wasm）: ホストの関数（`vscode-extension/src/wasm_host.ts` の `hostImports`）に頼む。
///
/// 値を返す関数は、ホストが結果を保留に置いて長さ（無ければ負）を返し、こちらが領域を確保してから
/// `host_take` で写させる（[`take`]）。
#[cfg(target_arch = "wasm32")]
mod host {
    use std::path::{Path, PathBuf};

    #[link(wasm_import_module = "arrow_host")]
    extern "C" {
        fn host_stat(ptr: *const u8, len: usize) -> i32;
        fn host_read(ptr: *const u8, len: usize) -> i32;
        fn host_read_dir(ptr: *const u8, len: usize) -> i32;
        fn host_realpath(ptr: *const u8, len: usize) -> i32;
        fn host_env_path(ptr: *const u8, len: usize, list: i32) -> i32;
        fn host_cwd() -> i32;
        fn host_python_lib_dirs() -> i32;
        fn host_take(dst: *mut u8);
    }

    /// ホストの失敗の戻り値（`wasm_host.ts` の `NOT_FOUND` / `OTHER_ERROR`）。
    const NOT_FOUND: i32 = -1;

    /// ホストが保留に置いた `len` バイトを受け取る。`len` が負なら `Err(len)`。
    fn take(len: i32) -> Result<Vec<u8>, i32> {
        if len < 0 {
            return Err(len);
        }
        // ⚠ 確保でメモリが伸びてもよい（ホストは `host_take` の中でメモリを取り直す）。
        let mut buf = vec![0u8; len as usize];
        unsafe { host_take(buf.as_mut_ptr()) };
        Ok(buf)
    }

    fn io_error(code: i32, what: &str, p: &Path) -> std::io::Error {
        let kind = if code == NOT_FOUND {
            std::io::ErrorKind::NotFound
        } else {
            std::io::ErrorKind::Other
        };
        std::io::Error::new(kind, format!("cannot {what} '{}'", p.display()))
    }

    /// パスを渡して長さを受け取る呼び出し。
    fn call(f: unsafe extern "C" fn(*const u8, usize) -> i32, p: &Path) -> i32 {
        let s = p.to_string_lossy();
        unsafe { f(s.as_ptr(), s.len()) }
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// `\n` 区切りの並び（空なら空）。
    fn lines(bytes: Vec<u8>) -> Vec<String> {
        let s = text(bytes);
        if s.is_empty() {
            return Vec::new();
        }
        s.split('\n').map(str::to_string).collect()
    }

    pub fn exists(p: &Path) -> bool {
        call(host_stat, p) != 0
    }

    pub fn is_dir(p: &Path) -> bool {
        call(host_stat, p) == 2
    }

    pub fn canonicalize(p: &Path) -> Option<PathBuf> {
        take(call(host_realpath, p)).ok().map(|b| PathBuf::from(text(b)))
    }

    pub fn read_dir(p: &Path) -> std::io::Result<Vec<PathBuf>> {
        let names = take(call(host_read_dir, p)).map_err(|c| io_error(c, "read directory", p))?;
        Ok(lines(names).into_iter().map(|n| p.join(n)).collect())
    }

    pub fn read(p: &Path) -> std::io::Result<Vec<u8>> {
        take(call(host_read, p)).map_err(|c| io_error(c, "read", p))
    }

    pub fn read_to_string(p: &Path) -> std::io::Result<String> {
        String::from_utf8(read(p)?)
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "stream did not contain valid UTF-8"))
    }

    /// 拡張は import の副産物を書き出さない（利用者のファイルを解析が書き換えない）。
    pub fn write(_p: &Path, _contents: &str) -> std::io::Result<()> {
        Ok(())
    }

    fn env(name: &str, list: bool) -> Option<Vec<PathBuf>> {
        let len = unsafe { host_env_path(name.as_ptr(), name.len(), list as i32) };
        take(len).ok().map(|b| lines(b).into_iter().map(PathBuf::from).collect())
    }

    pub fn env_path(name: &str) -> Option<PathBuf> {
        env(name, false).and_then(|v| v.into_iter().next())
    }

    pub fn env_paths(name: &str) -> Vec<PathBuf> {
        env(name, true).unwrap_or_default()
    }

    pub fn current_dir() -> Option<PathBuf> {
        take(unsafe { host_cwd() }).ok().map(|b| PathBuf::from(text(b)))
    }

    /// ⚠ ホストが**1 度だけ**調べる（`wasm_host.ts`）。ここでも保持する（CLI と同じく、解析ごとに変わらない）。
    pub fn python_lib_dirs() -> &'static [PathBuf] {
        use std::sync::OnceLock;
        static DIRS: OnceLock<Vec<PathBuf>> = OnceLock::new();
        DIRS.get_or_init(|| {
            take(unsafe { host_python_lib_dirs() })
                .map(|b| lines(b).into_iter().map(PathBuf::from).collect())
                .unwrap_or_default()
        })
    }
}
