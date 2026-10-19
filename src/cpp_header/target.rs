// target.rs — C の型の幅を決める OS（`ar_config.json` の `"os"`・ffi_value_exchange_plan.md 7-32）: TargetOs / target_os / apply_config_os。
//
// C の `long` と `wchar_t` の幅は OS で決まる: Windows（LLP64）は `long` = 4 バイト・`wchar_t` = UTF-16 の 2 バイト（符号なし）、
// Linux / macOS（LP64）は `long` = 8 バイト・`wchar_t` = UTF-32 の 4 バイト（符号付き）。
// ⚠ 以前は Arrow を作ったときの OS（`cfg!(windows)`）で決めていた。拡張（wasm）では `cfg!(windows)` が常に偽なので、Windows で
//   使っても Linux の幅になり、CLI と違う型スタブ（`wchar_t*` の要素型など）を作りえた。
// ⇒ ヘッダを読む前に `ar_config.json` の `"os"` から決めて（`apply_config_os`）、このスレッドの値として置く。書いていなければ
//   動いている OS（`import_fs::host_os`。拡張はホストに尋ねる）。書いた OS が動いている OS と違えば誤り（その OS の C の
//   ライブラリは読めない）。
// ⚠ スレッドごとの値にしてある（import と中継コードの生成は import した実行のスレッドで行う。単体テストが別々の OS を置ける）。
//   置いていないスレッド（非同期タスクのワーカー）は動いている OS を使うが、実行時は書いた OS が動いている OS と一致することを
//   確かめてある（`apply_config_os` の `check_host`）ので、どのスレッドでも同じ値になる。

use std::cell::Cell;

/// C の型の幅を決める OS。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetOs {
    Windows,
    Linux,
    MacOs,
}

impl TargetOs {
    /// `ar_config.json` の `"os"` の値。読めなければ誤り（黙って既定値にしない）。
    pub fn parse(name: &str) -> Result<TargetOs, String> {
        match name.trim().to_ascii_lowercase().as_str() {
            "windows" | "win32" => Ok(TargetOs::Windows),
            "linux" => Ok(TargetOs::Linux),
            "macos" | "darwin" => Ok(TargetOs::MacOs),
            other => Err(format!(
                "ar_config.json: unknown \"os\" value '{other}' (write \"windows\", \"linux\" or \"macos\")"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TargetOs::Windows => "windows",
            TargetOs::Linux => "linux",
            TargetOs::MacOs => "macos",
        }
    }

    pub fn is_windows(self) -> bool {
        self == TargetOs::Windows
    }

    /// C の `long` のビット数（Windows は 32・ほかは 64）。
    pub fn long_bits(self) -> u32 {
        if self.is_windows() { 32 } else { 64 }
    }

    /// `wchar_t` のバイト数（Windows は UTF-16 の 2・ほかは UTF-32 の 4）。
    pub fn wchar_bytes(self) -> usize {
        if self.is_windows() { 2 } else { 4 }
    }

    /// `wchar_t` が符号付きか（Windows は `unsigned short`・Linux / macOS は `int`）。
    pub fn wchar_signed(self) -> bool {
        !self.is_windows()
    }

    /// 動いている OS（CLI は Arrow を作った OS、拡張はホストの `process.platform`）。分からなければ `None`。
    pub fn host() -> Option<TargetOs> {
        crate::import_fs::host_os().and_then(|s| TargetOs::parse(&s).ok())
    }
}

thread_local! {
    static TARGET: Cell<Option<TargetOs>> = const { Cell::new(None) };
}

/// 今の C の型の幅に使う OS: `set_target_os`（`apply_config_os`）で置いたもの、無ければ動いている OS。
/// ⚠ 動いている OS も分からない（拡張のホストが答えない）ときは Windows（Arrow の C の連携が主に動く OS・既定の
///   `ar_config.json` も Windows）。
pub fn target_os() -> TargetOs {
    TARGET.with(|t| t.get()).or_else(TargetOs::host).unwrap_or(TargetOs::Windows)
}

/// このスレッドの OS を置く。
pub fn set_target_os(os: TargetOs) {
    TARGET.with(|t| t.set(Some(os)));
}

/// `ar_config.json` の `"os"`（`CppBuildConfig::os`）から OS を決めて置く。ヘッダを読む前に呼ぶ。
/// 書いていなければ動いている OS。値が読めなければ誤り。
/// `check_host`: 書いた OS が動いている OS と違えば誤り（C のライブラリはその OS のものしか読めない）。型スタブ（拡張も
/// ホストの OS で確かめる・CLI と同じ診断）と実行時が `true` で呼ぶ。動いている OS が分からなければ確かめない。
pub fn apply_config_os(cfg: &super::CppBuildConfig, check_host: bool) -> Result<TargetOs, String> {
    let os = match cfg.os.as_deref() {
        Some(name) => TargetOs::parse(name)?,
        None => TargetOs::host().unwrap_or(TargetOs::Windows),
    };
    if check_host {
        if let Some(host) = TargetOs::host() {
            if host != os {
                return Err(format!(
                    "ar_config.json says \"os\": \"{}\", but Arrow runs on {}; a C library built for another OS cannot be loaded",
                    os.name(),
                    host.name()
                ));
            }
        }
    }
    set_target_os(os);
    Ok(os)
}

/// `wchar_t` の文字列（`unit` バイト単位・NUL 終端）を UTF-16LE のバイト列にする（D-9: Arrow は `wchar_t` をどの OS でも
/// UTF-16LE の `bytes` で受け取る）。`unit` = 2 はそのまま、4（UTF-32）は UTF-16 に符号化し直す。Unicode のスカラー値でない
/// 値（サロゲート・0x10FFFF より大きい）は誤り。
///
/// # Safety
/// `p` は NUL で終わる `unit` バイト単位の列を指すこと。
pub unsafe fn wide_text_utf16le(p: *const u8, unit: usize) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut n = 0usize;
    loop {
        if unit == 2 {
            let c = std::ptr::read_unaligned(p.add(n * 2) as *const u16);
            if c == 0 {
                break;
            }
            out.extend_from_slice(&c.to_le_bytes());
        } else {
            let c = std::ptr::read_unaligned(p.add(n * 4) as *const u32);
            if c == 0 {
                break;
            }
            let ch = char::from_u32(c).ok_or_else(|| {
                format!("ValueError: a wchar_t string holds 0x{c:X}, which is not a Unicode character")
            })?;
            let mut buf = [0u16; 2];
            for u in ch.encode_utf16(&mut buf) {
                out.extend_from_slice(&u.to_le_bytes());
            }
        }
        n += 1;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_names_and_widths() {
        assert_eq!(TargetOs::parse("Windows").unwrap(), TargetOs::Windows);
        assert_eq!(TargetOs::parse("darwin").unwrap(), TargetOs::MacOs);
        assert!(TargetOs::parse("beos").unwrap_err().contains("unknown \"os\""));
        assert_eq!((TargetOs::Windows.long_bits(), TargetOs::Windows.wchar_bytes()), (32, 2));
        assert_eq!((TargetOs::Linux.long_bits(), TargetOs::Linux.wchar_bytes()), (64, 4));
    }

    /// 書いた OS が置かれ、動いている OS と違えば誤り（確かめるときだけ）
    #[test]
    fn config_os_is_applied_and_checked() {
        let mut cfg = super::super::CppBuildConfig::default();
        cfg.os = Some("linux".to_string());
        let host = TargetOs::host().expect("the CLI knows its OS");
        assert_eq!(apply_config_os(&cfg, false).unwrap(), TargetOs::Linux);
        assert_eq!(target_os(), TargetOs::Linux);
        if host != TargetOs::Linux {
            assert!(apply_config_os(&cfg, true).unwrap_err().contains("runs on"));
        }
        cfg.os = None;
        assert_eq!(apply_config_os(&cfg, true).unwrap(), host, "unwritten: the running OS");
        cfg.os = Some("plan9".to_string());
        assert!(apply_config_os(&cfg, false).is_err());
    }

    /// UTF-32 の `wchar_t`（Linux / macOS）は UTF-16LE に符号化し直す。サロゲート対になる文字も
    #[test]
    fn wide_text_becomes_utf16le() {
        let s32: Vec<u32> = "a\u{3042}\u{1F600}".chars().map(|c| c as u32).chain([0]).collect();
        let want: Vec<u8> = "a\u{3042}\u{1F600}".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(unsafe { wide_text_utf16le(s32.as_ptr() as *const u8, 4) }.unwrap(), want);
        let s16: Vec<u16> = "a\u{3042}".encode_utf16().chain([0]).collect();
        assert_eq!(unsafe { wide_text_utf16le(s16.as_ptr() as *const u8, 2) }.unwrap(), vec![0x61, 0, 0x42, 0x30]);
        let bad: [u32; 2] = [0xD800, 0];
        assert!(unsafe { wide_text_utf16le(bad.as_ptr() as *const u8, 4) }.is_err());
    }
}
