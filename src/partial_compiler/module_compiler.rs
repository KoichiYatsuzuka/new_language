/// `.arc` compiled module — writer, native compilation, and the runtime cache of embedded DLLs.
///
/// ⚠ The file format (and the reader) lives in `crate::arc_format`: the import processing reads the
///   embedded source with it, in the VS Code extension's wasm too (editor_import_resolution_plan.md 3-2).
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::llvm_codegen as codegen;
use super::stub_gen;
use crate::arc_format::{MAGIC, VERSION_V0, VERSION_V1};
use crate::ast::Stmt;
use crate::type_check::AstAnnotations;

// ---------------------------------------------------------------------------
// Thread-local cache: module_name → (exports, NativePayload)
// Populated by load_tlc(); consumed by exec.rs.
// ---------------------------------------------------------------------------

/// Payload stored in the native cache and embedded in .arc v1.
#[derive(Clone)]
pub enum NativePayload {
    /// v1: raw shared-library bytes, written to a temp file and loaded via libloading.
    Dll(Vec<u8>),
}

thread_local! {
    static NATIVE_CACHE: RefCell<HashMap<String, (Vec<codegen::FnExport>, NativePayload)>> =
        RefCell::new(HashMap::new());
}

/// Consume and return the cached native data for a module (if any).
pub fn take_native_bytes(module_name: &str) -> Option<(Vec<codegen::FnExport>, NativePayload)> {
    NATIVE_CACHE.with(|c| c.borrow_mut().remove(module_name))
}

/// Insert pre-compiled native data (DLL bytes) into the cache (used by `rs_loader`).
pub fn cache_native(module_name: &str, exports: Vec<codegen::FnExport>, dll_bytes: Vec<u8>) {
    NATIVE_CACHE.with(|c| c.borrow_mut().insert(
        module_name.to_string(),
        (exports, NativePayload::Dll(dll_bytes)),
    ));
}

// ── public API ────────────────────────────────────────────────────────────────

/// Compile `source` (already parsed into `stmts`) and write `.arc` + `.ars`
/// next to the original `source_path`.
///
/// If native compilation succeeds, the `.arc` is written as **version 1**
/// with the DLL bytes embedded inside it.  No separate `_tl.*` file is
/// created.  If `rustc` is unavailable or no functions are eligible, the
/// `.arc` is written as version 0 (source-only) and a warning is printed.
///
/// Returns the paths of the two output files (`.arc`, `.ars`) on success.
///
/// `annotations` は型検査が生成した AST 型解決層の注釈（#16 段階(c)）。codegen が
/// node-id 索引で解決型を引き、自前の型再導出を置き換えるために使う。
pub fn compile(
    source: &str,
    stmts: &[Stmt],
    source_path: &Path,
    annotations: &AstAnnotations,
) -> std::io::Result<(std::path::PathBuf, std::path::PathBuf)> {
    let stem = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("module");

    let parent = source_path.parent().unwrap_or(Path::new("."));

    let tlc_path = parent.join(format!("{stem}.arc"));
    let tls_path = parent.join(format!("{stem}.ars"));

    let stub = stub_gen::generate_stub(stmts);
    std::fs::write(&tls_path, &stub)?;

    // Attempt native compilation.
    match compile_native(stmts, annotations) {
        Ok((payload, exports)) => {
            let NativePayload::Dll(bytes) = &payload;
            write_tlc_native(source, stem, &exports, bytes, VERSION_V1, &tlc_path)?;
            println!(
                "NativeLib: {} function(s) (DLL) embedded in {}",
                exports.len(), tlc_path.display()
            );
        }
        Err(e) => {
            eprintln!("NativeLib: skipped ({e})");
            write_tlc_v0(source, stem, &tlc_path)?;
        }
    }

    Ok((tlc_path, tls_path))
}

/// Platform native-library extension (`dll` / `so` / `dylib`).
/// Used when extracting the embedded DLL to a temp file at runtime.
pub fn native_lib_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

/// 読み込む DLL の中身 `bytes` を一時ディレクトリに置き、そのパスを返す（copy_alloc_speed_plan.md 7-8）。
///
/// 名前は `{prefix}_{中身のハッシュ}.{ext}`。**同じ中身のファイルが既にあれば書き直さない**（更新日時だけ新しくする）。
/// 書いたばかりの DLL は読み込みのたびにウイルス対策の検査を受け、`LoadLibrary` が 1 回 0.6〜0.8 s かかる
/// （実測 2026-11-17・130 KB の DLL。変えていないファイルなら別のプロセスからでも約 30 ms。更新日時を変えても速いまま）。
/// 以前は実行のたびに `{stem}_tl.dll` を書き直していて、コンパイル済みモジュールを読むプロセスが毎回これを払っていた
/// （速度の調査のベンチで、6 区間の合計 0.56 s に対して起動が 0.7 s）。
///
/// - 中身ごとに名前が違うので、違う版のモジュール（違う exe が作った `.arc`）を同時に走らせても互いのファイルを
///   書き換えない（以前は、ほかのプロセスが読み込んでいる DLL は書けず、ツリーウォークに落ちていた）。
/// - 書くときは別名に書いてから名前を変える（ほかのプロセスが書きかけのファイルを読まない）。名前を変えられなかった
///   ときは、同じ中身を別のプロセスが先に置いた場合だけ成功とする。
/// - 新しく書いたときに、同じ `prefix` のファイルのうち 7 日以上使われていないもの（更新日時が古いもの）を消す。
/// - ⚠ **1 つのプロセスで同じパスを 2 回返さない**。DLL は同じパスを 2 回読み込むと同じ実体になり、中の大域
///   （IC のセル・クラスの表のセル）を 2 つのモジュールが共有してしまう（同じソースの 2 つのモジュール・同じプロセスの
///   別のインタープリタ〔`cargo test` のスレッド〕）。2 回目からは写しの番号を混ぜた名前の別のファイルにする
///   （以前はここで、読み込み中の DLL を書けずにツリーウォークに落ちていた）。
pub fn materialize_native_lib(prefix: &str, bytes: &[u8]) -> std::io::Result<std::path::PathBuf> {
    static HANDED_OUT: std::sync::Mutex<Vec<std::path::PathBuf>> = std::sync::Mutex::new(Vec::new());
    let fnv = |h: u64, b: u8| (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    let dir = std::env::temp_dir();
    let base = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| fnv(h, b));
    // ⚠ 毒された（ほかのスレッドが持ったまま panic した）ときも中身はパスの一覧なので、そのまま使ってよい
    let mut handed = HANDED_OUT.lock().unwrap_or_else(|e| e.into_inner());
    let mut copy: u64 = 0;
    loop {
        let hash = if copy == 0 { base } else { copy.to_le_bytes().iter().fold(base, |h, &b| fnv(h, b)) };
        let name = format!("{prefix}_{hash:016x}.{}", native_lib_ext());
        let path = dir.join(&name);
        if handed.contains(&path) {
            copy += 1;
            continue;
        }
        if place_native_lib(&dir, &name, bytes)? {
            remove_stale_native_libs(&dir, prefix, &name);
        }
        handed.push(path.clone());
        return Ok(path);
    }
}

/// `dir/name` に中身 `bytes` のファイルを置く。同じ中身が既にあれば書かずに更新日時だけ新しくする。
/// 書いたら `true`（`materialize_native_lib`）。
fn place_native_lib(dir: &Path, name: &str, bytes: &[u8]) -> std::io::Result<bool> {
    let path = dir.join(name);
    if same_contents(&path, bytes) {
        // ⚠ 失敗は無視してよい: 更新日時は古いファイルを消す目安にしか使わない（消されたら次に書き直すだけ）
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        return Ok(false);
    }
    let tmp = dir.join(format!("{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    if let Err(e) = std::fs::rename(&tmp, &path) {
        // ⚠ 片付けの失敗は無視してよい（一時ファイルが残るだけ）
        let _ = std::fs::remove_file(&tmp);
        if !same_contents(&path, bytes) {
            return Err(e);
        }
    }
    Ok(true)
}

fn same_contents(path: &Path, bytes: &[u8]) -> bool {
    match std::fs::read(path) {
        Ok(b) => b == bytes,
        Err(_) => false,
    }
}

/// `prefix` の DLL のうち、7 日以上使われていないものを消す（`materialize_native_lib`）。
fn remove_stale_native_libs(dir: &Path, prefix: &str, keep: &str) {
    const STALE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 60 * 60);
    let head = format!("{prefix}_");
    let tail = format!(".{}", native_lib_ext());
    // ⚠ ここの失敗は全部無視してよい: 片付けなので、読めない・消せない（ほかのプロセスが読み込んでいる）ファイルは残すだけ
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(n) = file_name.to_str() else { continue };
        if n == keep || !n.starts_with(&head) || !n.ends_with(&tail) {
            continue;
        }
        // 名前の残りが 16 桁の 16 進数のものだけ（`{prefix}_x` という別のモジュールの DLL を消さない）
        let mid = &n[head.len()..n.len() - tail.len()];
        if mid.len() != 16 || !mid.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// 一時ディレクトリの下に、**このプロセスだけが使う**作業ディレクトリ（`<prefix>_<pid>_<連番>`）を作る。
///
/// ⚠ 以前は cpp-dll の中継のビルド（`_tl_cpp_bridge.rs` と rustc の中間）・`import[rs]` の cargo のプロジェクト
///   （`ar_rs_<crate>`）・`--compile` の clang の入出力が一時ディレクトリの**固定の名前**を使っていたので、同時に走る
///   arrow.exe どうし（ゲートの並列実行・別のセッション）が同じファイルを書き合い、rustc のリンクの失敗・cargo の
///   プロジェクトが消えるなどで落ちた（gate_speed_plan.md 2-1）。
/// ⚠ **読み込む DLL**（`.arc` から取り出す DLL・cpp-dll の中継 DLL）はここに置かない。中身のハッシュの名前で
///   プロセスをまたいで使い回す `materialize_native_lib`（copy_alloc_speed_plan.md 7-8）が置く。プロセスごとに
///   書き直すと、読み込みのたびにウイルス対策の検査を受ける。
///
/// 前に作った同じ接頭辞のディレクトリのうち 1 時間より古いものは、ここで消す（途中で止まったプロセスの残り）。
/// ⚠ 消すのは失敗しても構わない（使用中のものは消せずに残るだけ）。1 時間は、ビルドの途中のディレクトリを消さない
/// ための余裕。
pub fn private_temp_dir(prefix: &str) -> std::io::Result<PathBuf> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static SEQ: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir();
    sweep_stale_private_dirs(&root, prefix);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = root.join(format!("{prefix}_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// `private_temp_dir` の後始末: `<prefix>_<数字>_<数字>` の形で、更新が 1 時間より前のディレクトリを消す（失敗は無視）。
fn sweep_stale_private_dirs(root: &Path, prefix: &str) {
    const STALE: std::time::Duration = std::time::Duration::from_secs(60 * 60);
    let Ok(entries) = std::fs::read_dir(root) else { return };
    let head = format!("{prefix}_");
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(rest) = name.strip_prefix(&head) else { continue };
        // `<pid>_<連番>` の形のものだけ（同じ接頭辞で始まる別の名前のディレクトリは消さない。`import[rs]` は接頭辞にクレートの名前を含むので、別のクレートの接頭辞の先頭が重なることがある）
        let mut parts = rest.split('_');
        let is_ours = matches!((parts.next(), parts.next(), parts.next()), (Some(a), Some(b), None)
            if !a.is_empty() && !b.is_empty() && a.bytes().all(|c| c.is_ascii_digit()) && b.bytes().all(|c| c.is_ascii_digit()));
        if !is_ours {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Load a `.arc` file and return `(module_name, source_text)`.
///
/// If the file is v1, the embedded native data is placed in the
/// thread-local `NATIVE_CACHE` so that `exec.rs` can pick it up when
/// the module is later imported.
///
/// ⚠ 「`.arc` が古いかどうか」の判定には使わない（古いと判った後に DLL がキャッシュに残る＝
///   新しいソース × 古い DLL という最悪の組み合わせになる）。判定は形式を読むだけの
///   `crate::arc_format::read` で行う（`parser/imports/ar_modules.rs`）。
pub fn load_tlc(path: &Path) -> std::io::Result<(String, String)> {
    let arc = crate::arc_format::read(path)?;
    let name = arc.module_name;

    if let Some((exports, dll)) = arc.native {
        let exports = exports
            .into_iter()
            .map(|(name, n_params)| codegen::FnExport { name, n_params })
            .collect();
        NATIVE_CACHE.with(|c| {
            c.borrow_mut().insert(name.clone(), (exports, NativePayload::Dll(dll)));
        });
    }

    Ok((name, arc.source))
}

// ── native compilation ────────────────────────────────────────────────────────

/// Compile eligible functions to a `NativePayload`.
///
/// Emits LLVM IR text via `llvm_codegen`, then compiles it to a shared library
/// with the external `clang` driver.
fn compile_native(
    stmts: &[Stmt],
    annotations: &AstAnnotations,
) -> Result<(NativePayload, Vec<codegen::FnExport>), String> {
    let (llvm_ir, exports) = codegen::generate_llvm_module(stmts, annotations)
        .ok_or_else(|| "no codegen-eligible functions".to_string())?;

    let fn_names: Vec<&str> = exports.iter().map(|e| e.name.as_str()).collect();
    eprintln!("NativeLib(clang): compiling {} function(s): {}", fn_names.len(), fn_names.join(", "));

    // ⚠ このプロセスだけの作業ディレクトリ（同時に走る `--compile` が同じ名前を書き合わないように・`private_temp_dir`）
    let tmp_dir  = private_temp_dir("ar_native_module")
        .map_err(|e| format!("cannot create a temp dir: {e}"))?;
    let ll_path  = tmp_dir.join("ar_native_module.ll");
    let ext      = native_lib_ext();
    let dll_path = tmp_dir.join(format!("ar_native_module.{ext}"));

    std::fs::write(&ll_path, &llvm_ir)
        .map_err(|e| format!("cannot write LLVM IR: {e}"))?;

    // 開発用フック（#16 段階(c)）: `AR_DUMP_LL=<path>` を指定すると生成 IR をそこへも保存する。
    // codegen 変更が IR を変えていないこと（byte-identical）を差分で検証するために使う。
    if let Ok(dump) = std::env::var("AR_DUMP_LL") {
        if !dump.is_empty() {
            let _ = std::fs::write(&dump, &llvm_ir);
        }
    }

    let result = invoke_clang(&ll_path, &dll_path);
    let dll_bytes = result.and_then(|()| {
        std::fs::read(&dll_path).map_err(|e| format!("cannot read compiled DLL: {e}"))
    });
    // clang の中間・出力ごと消す（読み込んでいないので消せる）
    let _ = std::fs::remove_dir_all(&tmp_dir);
    Ok((NativePayload::Dll(dll_bytes?), exports))
}

/// Invoke `clang -O3 -shared` to compile `ll_path` → `dll_path`.
fn invoke_clang(ll_path: &Path, dll_path: &Path) -> Result<(), String> {
    let out_str = dll_path.to_str().unwrap_or("output");
    let in_str  = ll_path.to_str().unwrap_or("");
    let mut args: Vec<&str> = vec!["-O3", "-shared", "-o", out_str, in_str];
    #[cfg(not(target_os = "windows"))]
    args.push("-fPIC");
    #[cfg(target_os = "windows")]
    args.extend_from_slice(&["-Wno-dll-attribute-on-redeclaration"]);

    // Try clang from PATH first; fall back to llvm.path in ar_config.json.
    let clang_exe = if Command::new("clang").arg("--version").output()
        .is_ok_and(|o| o.status.success())
    {
        std::path::PathBuf::from("clang")
    } else {
        let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };
        let from_config = std::env::current_dir().ok()
            .and_then(|d| std::fs::read_to_string(d.join("ar_config.json")).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|j| j.get("llvm")?.get("path")?.as_str().map(|s| s.to_string()))
            .map(|p| std::path::PathBuf::from(p).join("bin").join(format!("clang{ext}")));
        from_config.filter(|p| p.exists()).unwrap_or_else(|| std::path::PathBuf::from("clang"))
    };

    let output = Command::new(&clang_exe).args(&args).output();
    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!("clang failed:\n{}", String::from_utf8_lossy(&out.stderr))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
            Err("clang not found in PATH (or llvm.path in ar_config.json)".to_string()),
        Err(e) => Err(format!("cannot run clang: {e}")),
    }
}

// ── writers ───────────────────────────────────────────────────────────────────

/// `[u32 LE length][bytes]` を書き出す。
fn write_len_prefixed(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// ソーステキストのみを埋め込んだ v0 形式の `.arc` ファイルを書き出す。
fn write_tlc_v0(source: &str, module_name: &str, path: &Path) -> std::io::Result<()> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION_V0.to_le_bytes());
    write_len_prefixed(&mut buf, module_name.as_bytes());
    write_len_prefixed(&mut buf, source.as_bytes());
    std::fs::write(path, buf)
}

/// ネイティブペイロード(DLL バイト列)付きの `.arc` を書き出す。
fn write_tlc_native(
    source: &str,
    module_name: &str,
    exports: &[codegen::FnExport],
    payload: &[u8],
    version: u32,
    path: &Path,
) -> std::io::Result<()> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&version.to_le_bytes());
    write_len_prefixed(&mut buf, module_name.as_bytes());
    write_len_prefixed(&mut buf, source.as_bytes());
    buf.extend_from_slice(&(exports.len() as u32).to_le_bytes());
    for exp in exports {
        write_len_prefixed(&mut buf, exp.name.as_bytes());
        buf.extend_from_slice(&(exp.n_params as u32).to_le_bytes());
    }
    write_len_prefixed(&mut buf, payload);
    std::fs::write(path, buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ar_native_lib_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 同じ中身なら書き直さない・違う中身なら書き直す（7-8）
    #[test]
    fn place_native_lib_rewrites_only_changed_contents() {
        let d = scratch("place");
        assert!(place_native_lib(&d, "m_tl_x.dll", b"abc").unwrap(), "first placement writes");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(d.join("m_tl_x.dll")).unwrap().set_modified(old).unwrap();
        assert!(!place_native_lib(&d, "m_tl_x.dll", b"abc").unwrap(), "same contents are not rewritten");
        let touched = std::fs::metadata(d.join("m_tl_x.dll")).unwrap().modified().unwrap();
        assert!(touched > old, "reuse refreshes the modification time");
        assert!(place_native_lib(&d, "m_tl_x.dll", b"abcd").unwrap(), "changed contents are rewritten");
        assert_eq!(std::fs::read(d.join("m_tl_x.dll")).unwrap(), b"abcd");
        let left: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left.len(), 1, "no temporary file is left: {left:?}");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 古いものだけ・同じ prefix の 16 桁のハッシュの名前だけを消す
    #[test]
    fn remove_stale_native_libs_keeps_recent_and_foreign_files() {
        let d = scratch("stale");
        let ext = native_lib_ext();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 24 * 60 * 60);
        let names = [
            format!("m_tl_{:016x}.{ext}", 1),       // 古い → 消す
            format!("m_tl_{:016x}.{ext}", 2),       // 新しい → 残す
            format!("m_tl_{:016x}.{ext}", 3),       // keep（今書いたもの）→ 残す
            format!("m_tl_b_tl_{:016x}.{ext}", 4),  // 別のモジュール（`m_tl_b`）→ 残す
            format!("n_tl_{:016x}.{ext}", 5),       // 別の prefix → 残す
        ];
        for (i, n) in names.iter().enumerate() {
            std::fs::write(d.join(n), b"x").unwrap();
            if i != 1 {
                std::fs::File::options().write(true).open(d.join(n)).unwrap().set_modified(old).unwrap();
            }
        }
        remove_stale_native_libs(&d, "m_tl", &names[2]);
        for (i, n) in names.iter().enumerate() {
            assert_eq!(d.join(n).exists(), i != 0, "{n}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 1 つのプロセスで同じ中身を 2 回置くと別のパスになる（DLL の実体を共有しない）
    #[test]
    fn materialize_native_lib_never_hands_out_a_path_twice() {
        let prefix = format!("ar_test_materialize_{}", std::process::id());
        let bytes = b"not really a dll".to_vec();
        let a = materialize_native_lib(&prefix, &bytes).unwrap();
        let b = materialize_native_lib(&prefix, &bytes).unwrap();
        assert_ne!(a, b);
        assert_eq!(std::fs::read(&a).unwrap(), bytes);
        assert_eq!(std::fs::read(&b).unwrap(), bytes);
        let _ = std::fs::remove_file(&a);
        let _ = std::fs::remove_file(&b);
    }
}

#[cfg(test)]
mod private_temp_dir_tests {
    use super::{private_temp_dir, sweep_stale_private_dirs};

    /// 呼ぶたびに別のディレクトリ（同じプロセスの中でも取り合わない）
    #[test]
    fn each_call_gets_its_own_dir() {
        let a = private_temp_dir("ar_ptd_test").unwrap();
        let b = private_temp_dir("ar_ptd_test").unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// 後始末は `<prefix>_<数字>_<数字>` の形で古いものだけを消す（新しいもの・別の名前は残す）
    #[test]
    fn sweep_removes_only_stale_dirs_of_the_same_shape() {
        let root = std::env::temp_dir().join(format!("ar_ptd_sweep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let fresh = root.join("ar_x_1_0");
        let other = root.join("ar_x_foo");
        std::fs::create_dir_all(&fresh).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        sweep_stale_private_dirs(&root, "ar_x");
        // 作ったばかりなので古くない
        assert!(fresh.is_dir());
        assert!(other.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }
}
