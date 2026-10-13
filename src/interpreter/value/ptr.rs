// value/ptr.rs — 外部のメモリを指す値 `Ptr`（ffi_value_exchange_plan.md 3-2・D-3・D-11・D-15）: PtrHandle / PtrOwner / ReleaseFn / PtrKeepAlive。
//
// C などのポインタを、中身を展開せずに Arrow の値として持つ。できるのは比較・C へ渡し返す・（所有していれば）
// `release()`・長さと型が分かるものの展開（`p.to_list()` / `p.to_bytes()`・3-3）。**アドレスの取得・加算・整数からの生成・
// 中身の書き込みはできない**（D-3・D-11）。だから表示にもアドレスを出さない。
//
// 所有は 3 種類（`PtrOwner`）:
//   - `Owned`    … 解放関数を持つ（`PtrProtocol` の `release`・D-15）。最後の参照がなくなったとき（`Drop`）か
//                  `release()` で、**1 回だけ**呼ぶ
//   - `Static`   … flags に STATIC（DLL のアンロードまで有効で中身も変わらない）。解放しない
//   - `Borrowed` … 生のヘッダの戻り値（D-7）。Arrow は解放しない
// ⚠ コピーは共有（`Arc`）。`deep_clone` も同じ `PtrHandle` を共有するので、非同期タスクへ渡しても解放は 1 回。
// ⚠ 元の DLL への参照（`lib`）を持ち、`Ptr` が残っている間は DLL をアンロードしない（解放関数は DLL の中にある）。
// ⚠ C の側で解放されたとき（`PtrProtocol*` の `address` が 0 に書き戻された・4-5）は `invalidate()` し、以後の使用は
//   エラーにする（解放関数は呼ばない）。
// ⚠ NULL の `Ptr` は作らない。NULL は `None`（D-8）なので、アドレス 0 で作ろうとしたら誤り。
// ⚠ 作るのはフェーズ 4 のブリッジ（4-3 の生のポインタ・4-5 の `PtrProtocol`）。それまでは単体テストだけが作るので、
//   使われない項目の警告を止めておく。

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

/// 指す先を持つ DLL を生かしておくための参照（cpp の `NativeLibWrapper`・cs-dll / rs のライブラリなど、ブリッジごとに型が違う）。
pub type PtrKeepAlive = Arc<dyn std::any::Any + Send + Sync>;

/// `PtrProtocol` の flags（D-1・D-15）。⚠ 共有ヘッダ `bridge/ptr_protocol/ar_ptr_protocol.h`（`AR_PTR_*`）と
/// C# の写し `ArPtrProtocol.cs` と同じ値にすること（単体テスト `protocol_files_agree` が突き合わせる）。
pub const PTR_FLAG_CONST: u32 = 1;
/// 文字データが NUL 終端である（`length` は NUL を含まない）。
pub const PTR_FLAG_NUL_TERMINATED: u32 = 2;
/// DLL のアンロードまで有効で中身も変わらない。`release` が NULL でよいのはこのときだけ（D-15）。
pub const PTR_FLAG_STATIC: u32 = 4;

/// `ArPtrProtocol`（`bridge/ptr_protocol/ar_ptr_protocol.h`・4-1）の写し。C / C# と**同じ並び**（64 ビットで 48 バイト）。
/// ⚠ 並びを変えるときは 3 つ（C のヘッダ・C# の写し・これ）を同時に直す（単体テスト `protocol_layout_matches_the_header`）。
/// 受け渡しは 4-4 / 4-5（`Ptr` との変換）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PtrProtocolRaw {
    /// 先頭のアドレス（NULL は 0）。
    pub address: u64,
    /// 要素の型名（NUL 終端の静的な文字列・D-12）。
    pub elem_type: *const std::ffi::c_char,
    pub flags: u32,
    /// 0（並びを C# と揃えるための明示の詰め物）。
    pub _reserved: u32,
    /// 要素の数（単体は 1。NUL 終端で数えない場合は -1）。
    pub length: i64,
    /// 解放関数。`None` は STATIC のときだけ（D-15）。
    pub release: Option<ReleaseFn>,
    pub release_ctx: *mut std::ffi::c_void,
}

/// 解放関数（`PtrProtocol` の `release`）。`ctx` は確保した側が渡した `PtrProtocol.release_ctx`（C の構造体のフィールド）。
pub type ReleaseFn = unsafe extern "C" fn(ctx: *mut std::ffi::c_void, address: u64);

/// 誰がメモリを解放するか。
#[derive(Debug, Clone, Copy)]
pub enum PtrOwner {
    /// Arrow が解放する（最後の参照がなくなったとき、または `release()`）。
    /// ⚠ `ctx` はアドレスとして持つ（`*mut c_void` は `Send` でないため。中身には触らず、解放関数へ返すだけ）。
    Owned { release: ReleaseFn, ctx: usize },
    /// DLL のアンロードまで有効（STATIC）。解放しない。
    Static,
    /// 借用（生のヘッダの戻り値・D-7）。解放しない。
    Borrowed,
}

/// 指す先がまだ使えるか。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtrState {
    Live,
    /// Arrow が `release()` で解放した。
    Released,
    /// C の側で解放された（4-5）。
    Invalidated,
}

/// `Value::Ptr` の中身。
pub struct PtrHandle {
    address: u64,
    /// 要素の型名（D-12。`"int32"` / `"float64"` / `"uint8"` / 構造体名 / `"void"`）。Arrow の型への対応は 3-3。
    elem_type: String,
    flags: u32,
    /// 要素数（`PtrProtocol` の `length`）。生のポインタ（D-7）は長さが分からないので `None`。
    length: Option<i64>,
    owner: PtrOwner,
    state: Mutex<PtrState>,
    /// 解放関数・指す先を持つ DLL。`Ptr` が残っている間はアンロードしない（持っているだけで読まない）。
    #[allow(dead_code)]
    lib: Option<PtrKeepAlive>,
}

impl PtrHandle {
    /// 作る。⚠ アドレス 0（NULL）は `None` で表すので誤り（D-8）。
    pub fn new(
        address: u64,
        elem_type: impl Into<String>,
        flags: u32,
        length: Option<i64>,
        owner: PtrOwner,
        lib: Option<PtrKeepAlive>,
    ) -> Result<Self, String> {
        let elem_type = elem_type.into();
        if address == 0 {
            return Err(format!("ValueError: a NULL pointer cannot be a Ptr[{elem_type}] (NULL is None)"));
        }
        if let Some(n) = length {
            if n < 0 {
                return Err(format!("ValueError: Ptr[{elem_type}] has a negative length ({n})"));
            }
        }
        if matches!(owner, PtrOwner::Static) && flags & PTR_FLAG_STATIC == 0 {
            return Err(format!("ValueError: Ptr[{elem_type}] is not released by Arrow, but its flags do not say STATIC"));
        }
        Ok(PtrHandle { address, elem_type, flags, length, owner, state: Mutex::new(PtrState::Live), lib })
    }

    pub fn elem_type(&self) -> &str {
        &self.elem_type
    }

    pub fn flags(&self) -> u32 {
        self.flags
    }

    pub fn length(&self) -> Option<i64> {
        self.length
    }

    pub fn owner(&self) -> PtrOwner {
        self.owner
    }

    fn state(&self) -> PtrState {
        *self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// まだ使えるか（解放・無効化されていないか）。
    pub fn is_live(&self) -> bool {
        self.state() == PtrState::Live
    }

    /// C へ渡すアドレス。解放・無効化された後は誤り。⚠ Arrow の値としては見せない（D-3）。
    pub fn address(&self) -> Result<u64, String> {
        match self.state() {
            PtrState::Live => Ok(self.address),
            dead => Err(self.dead_error(dead)),
        }
    }

    /// 解放・無効化された後に使ったときのメッセージ。
    /// ⚠ 状態は引数で受ける（ここで `state` を読み直さない）。`release()` は錠を持ったまま呼ぶので、
    ///   読み直すと同じ `Mutex` を二重に取って止まる（一度踏んだ）。
    fn dead_error(&self, state: PtrState) -> String {
        match state {
            PtrState::Released => format!("ValueError: Ptr[{}] was already released", self.elem_type),
            PtrState::Invalidated => {
                format!("ValueError: Ptr[{}] was freed by the library and can no longer be used", self.elem_type)
            }
            PtrState::Live => unreachable!("dead_error on a live Ptr"),
        }
    }

    /// `p.release()`。所有しているメモリを今すぐ解放する（`Drop` を待たない）。
    /// 所有していない・もう解放した・C の側で解放されたものは誤り（黙って何もしない、にはしない）。
    pub fn release(&self) -> Result<(), String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match (*state, self.owner) {
            (PtrState::Live, PtrOwner::Owned { release, ctx }) => {
                *state = PtrState::Released;
                // SAFETY: 解放関数と ctx は確保した側が `PtrProtocol` で渡したもの。状態を先に Released にして
                // あるので 2 回は呼ばない。DLL は `lib` が保持している。
                unsafe { release(ctx as *mut std::ffi::c_void, self.address) };
                Ok(())
            }
            (PtrState::Live, PtrOwner::Static) => {
                Err(format!("ValueError: Ptr[{}] points to static memory and cannot be released", self.elem_type))
            }
            (PtrState::Live, PtrOwner::Borrowed) => Err(format!(
                "ValueError: Ptr[{}] is borrowed from the library; Arrow does not own its memory and cannot release it",
                self.elem_type
            )),
            (dead @ (PtrState::Released | PtrState::Invalidated), _) => Err(self.dead_error(dead)),
        }
    }

    /// C の側で解放されたことを記録する（`PtrProtocol*` の `address` が 0 に書き戻された・4-5）。解放関数は呼ばない。
    #[allow(dead_code)]
    pub fn invalidate(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state == PtrState::Live {
            *state = PtrState::Invalidated;
        }
    }

    /// `p.to_list()`（3-3・D-10）。指す先の要素を Arrow の値へコピーする（展開した `list` は元のポインタとつながらない）。
    ///
    /// ⚠ 要素の型は作られたときの型（`elem_type`）で読む。数値だけ。構造体の要素はまだ展開できない（4-6）、
    ///   `void` は読めない、長さの分からない生のポインタ（D-7）も読めない。どれも誤り。
    /// ⚠ 中身を信じる根拠は `PtrProtocol` の規約（D-15: Arrow が参照している間は有効）。
    pub fn read_list(&self) -> Result<Vec<super::Value>, String> {
        let Some(width) = num_width(&self.elem_type) else {
            return Err(match self.elem_type.as_str() {
                "void" => "TypeError: Ptr[void] has no element type to read; convert it first with `p mustbe Ptr[T]`"
                    .to_string(),
                other => format!("TypeError: expanding Ptr[{other}] into a list of structs is not supported yet"),
            });
        };
        let len = self.known_length()?;
        let addr = self.address()?;
        (0..len)
            .map(|i| {
                // SAFETY: `PtrProtocol` が `length` 個の要素が `address` から並ぶと保証する（D-15）。並びの境界は
                // 揃っていないかもしれないので `read_unaligned`。
                unsafe { read_num(&self.elem_type, (addr as usize + i * width) as *const u8) }
            })
            .collect()
    }

    /// `p.to_bytes()`（3-3・D-9）。文字データ（`int8` / `uint8` / `uint16`）を `bytes` へコピーする。長さが無くても
    /// NUL 終端（flags）なら NUL の手前まで。`uint16`（`wchar_t` / char16_t）は UTF-16LE のバイト列になる。
    pub fn read_bytes(&self) -> Result<Vec<u8>, String> {
        let width = match self.elem_type.as_str() {
            "int8" | "uint8" => 1,
            "uint16" => 2,
            other => {
                return Err(format!(
                    "TypeError: only a Ptr to character data (int8 / uint8 / uint16) can be copied into bytes, not Ptr[{other}]; use to_list()"
                ))
            }
        };
        let addr = self.address()?;
        let count = match self.length {
            Some(n) => n as usize,
            None if self.flags & PTR_FLAG_NUL_TERMINATED != 0 => {
                // SAFETY: NUL 終端と flags が言っている（D-15）。
                let mut n = 0usize;
                while unsafe { std::ptr::read_unaligned((addr as usize + n * width) as *const u8) } != 0
                    || (width == 2
                        && unsafe { std::ptr::read_unaligned((addr as usize + n * width + 1) as *const u8) } != 0)
                {
                    n += 1;
                }
                n
            }
            None => return Err(self.no_length_error()),
        };
        // SAFETY: `count` 個の要素が並ぶ（`length` か NUL 終端・D-15）。
        Ok(unsafe { std::slice::from_raw_parts(addr as *const u8, count * width) }.to_vec())
    }

    fn known_length(&self) -> Result<usize, String> {
        self.length.map(|n| n as usize).ok_or_else(|| self.no_length_error())
    }

    fn no_length_error(&self) -> String {
        format!(
            "ValueError: Ptr[{}] has no length (it came from a raw pointer), so it cannot be expanded",
            self.elem_type
        )
    }

    /// 表示（`<Ptr[int32]>`）。⚠ アドレスは出さない（D-3: アドレスを取り出す手段を作らない）。
    pub fn display(&self) -> String {
        let len = match self.length {
            Some(n) => format!(" len={n}"),
            None => String::new(),
        };
        let state = match self.state() {
            PtrState::Live => "",
            PtrState::Released => " released",
            PtrState::Invalidated => " freed by the library",
        };
        format!("<Ptr[{}]{len}{state}>", self.elem_type)
    }
}

/// 数値の要素 1 つの幅（バイト）。数値でなければ `None`。
fn num_width(elem: &str) -> Option<usize> {
    Some(match elem {
        "int8" | "uint8" => 1,
        "int16" | "uint16" => 2,
        "int32" | "uint32" | "float32" => 4,
        "int64" | "uint64" | "float64" => 8,
        _ => return None,
    })
}

/// 数値の要素 1 つを読む（符号拡張・`float32` は `float` へ広げる）。`uint64` の 2^63 以上は `int` に入らないので
/// OverflowError（1-6 と同じ）。
///
/// # Safety
/// `p` から `num_width(elem)` バイトが読めること。
unsafe fn read_num(elem: &str, p: *const u8) -> Result<super::Value, String> {
    use super::Value;
    use std::ptr::read_unaligned as rd;
    Ok(match elem {
        "int8" => Value::Int(rd(p as *const i8) as i64),
        "uint8" => Value::Int(rd(p) as i64),
        "int16" => Value::Int(rd(p as *const i16) as i64),
        "uint16" => Value::Int(rd(p as *const u16) as i64),
        "int32" => Value::Int(rd(p as *const i32) as i64),
        "uint32" => Value::Int(rd(p as *const u32) as i64),
        "int64" => Value::Int(rd(p as *const i64)),
        "uint64" => {
            let v = rd(p as *const u64);
            Value::Int(i64::try_from(v).map_err(|_| format!("OverflowError: {}", super::U64_OVERFLOW_MSG))?)
        }
        "float32" => Value::Float(rd(p as *const f32) as f64),
        "float64" => Value::Float(rd(p as *const f64)),
        other => unreachable!("read_num on a non-numeric element '{other}' (num_width rejects it first)"),
    })
}

/// 型の注釈 `Ptr` / `Ptr[T]` が値に当てはまるか（`is` / `mustbe` / `case`・宣言した型の照合・3-3）。`Ptr` の注釈でなければ `None`。
///
/// - `Ptr` / `Ptr[void]` はどの `Ptr` にも当てはまる（どの `Ptr` も `Ptr[void]` として渡せる）
/// - `Ptr[T]` は**元の要素型**が `T` の `Ptr` だけ（`Ptr[void]` として受け取っても、作られたときの型を覚えている。
///   だから `p mustbe Ptr[Db]` は元が `Db` のときだけ成功する・D-3）。モジュールの名前で修飾した構造体（`m.Db`）は
///   最後の名前で比べる（要素の型名は C のヘッダの名前・D-12）
pub(crate) fn ptr_ann_matches(ann: &str, val: &super::Value) -> Option<bool> {
    let elem = if ann == "Ptr" {
        None
    } else {
        Some(ann.strip_prefix("Ptr[")?.strip_suffix(']')?)
    };
    let super::Value::Ptr(p) = val else {
        return Some(false);
    };
    let Some(elem) = elem.map(|e| e.split_whitespace().collect::<String>()) else {
        return Some(true);
    };
    let bare = elem.rsplit('.').next().unwrap_or(&elem);
    Some(elem == "void" || elem == p.elem_type || bare == p.elem_type)
}

impl Drop for PtrHandle {
    /// 最後の参照がなくなったら、所有していてまだ解放していないメモリを解放する（1 回だけ）。
    fn drop(&mut self) {
        let state = *self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        if let (PtrState::Live, PtrOwner::Owned { release, ctx }) = (state, self.owner) {
            // SAFETY: `release()` と同じ。`&mut self` なのでほかに使っている者はいない。
            unsafe { release(ctx as *mut std::ffi::c_void, self.address) };
        }
    }
}

impl std::fmt::Debug for PtrHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 呼ばれた回数を `ctx` の指す `AtomicUsize` に数える解放関数（テストごとに別の数え先を使う）。
    unsafe extern "C" fn count_release(ctx: *mut std::ffi::c_void, address: u64) {
        assert_eq!(address, 0x1000);
        (*(ctx as *const AtomicUsize)).fetch_add(1, Ordering::SeqCst);
    }

    fn owned(counter: &AtomicUsize) -> PtrHandle {
        let ctx = counter as *const AtomicUsize as usize;
        PtrHandle::new(0x1000, "int32", 0, Some(3), PtrOwner::Owned { release: count_release, ctx }, None).unwrap()
    }

    #[test]
    fn drop_releases_once_after_the_last_copy() {
        let n = AtomicUsize::new(0);
        let a = Arc::new(owned(&n));
        let b = Arc::clone(&a);
        drop(a);
        assert_eq!(n.load(Ordering::SeqCst), 0, "a copy is still alive");
        drop(b);
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn explicit_release_is_not_repeated_by_drop() {
        let n = AtomicUsize::new(0);
        let p = owned(&n);
        p.release().unwrap();
        assert_eq!(n.load(Ordering::SeqCst), 1);
        assert!(p.release().unwrap_err().contains("already released"));
        assert!(p.address().is_err());
        assert!(p.display().contains("released"));
        drop(p);
        assert_eq!(n.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn invalidated_pointer_is_not_released() {
        let n = AtomicUsize::new(0);
        let p = owned(&n);
        p.invalidate();
        assert!(p.address().unwrap_err().contains("freed by the library"));
        assert!(p.release().is_err());
        drop(p);
        assert_eq!(n.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn borrowed_and_static_cannot_be_released() {
        let b = PtrHandle::new(0x1000, "void", 0, None, PtrOwner::Borrowed, None).unwrap();
        assert!(b.release().unwrap_err().contains("borrowed"));
        assert_eq!(b.address().unwrap(), 0x1000, "a failed release keeps the pointer usable");
        let s = PtrHandle::new(0x1000, "uint8", PTR_FLAG_STATIC, Some(0), PtrOwner::Static, None).unwrap();
        assert!(s.release().unwrap_err().contains("static"));
    }

    #[test]
    fn invalid_construction_is_an_error() {
        assert!(PtrHandle::new(0, "int32", 0, Some(1), PtrOwner::Borrowed, None).unwrap_err().contains("NULL"));
        assert!(PtrHandle::new(1, "int32", 0, Some(-1), PtrOwner::Borrowed, None).is_err());
        // STATIC と書いていないのに解放しない（D-15）
        assert!(PtrHandle::new(1, "int32", 0, Some(1), PtrOwner::Static, None).is_err());
    }

    #[test]
    fn display_never_shows_the_address() {
        let p = PtrHandle::new(0xdead_beef, "float64", 0, Some(2), PtrOwner::Borrowed, None).unwrap();
        assert_eq!(p.display(), "<Ptr[float64] len=2>");
        let q = PtrHandle::new(0xdead_beef, "Db", 0, None, PtrOwner::Borrowed, None).unwrap();
        assert_eq!(q.display(), "<Ptr[Db]>");
    }

    fn borrowed_over<T>(buf: &[T], elem: &str, length: Option<i64>, flags: u32) -> PtrHandle {
        PtrHandle::new(buf.as_ptr() as u64, elem, flags, length, PtrOwner::Borrowed, None).unwrap()
    }

    #[test]
    fn to_list_reads_each_width() {
        use super::super::Value;
        let ints: [i32; 3] = [1, -2, 3];
        let got = borrowed_over(&ints, "int32", Some(3), 0).read_list().unwrap();
        assert!(matches!(got.as_slice(), [Value::Int(1), Value::Int(-2), Value::Int(3)]), "{got:?}");
        let bytes: [u8; 2] = [0xff, 7];
        let got = borrowed_over(&bytes, "int8", Some(2), 0).read_list().unwrap();
        assert!(matches!(got.as_slice(), [Value::Int(-1), Value::Int(7)]), "sign-extends int8: {got:?}");
        let floats: [f32; 1] = [0.5];
        let got = borrowed_over(&floats, "float32", Some(1), 0).read_list().unwrap();
        assert!(matches!(got.as_slice(), [Value::Float(f)] if *f == 0.5));
        let big: [u64; 1] = [u64::MAX];
        let e = borrowed_over(&big, "uint64", Some(1), 0).read_list().unwrap_err();
        assert!(e.starts_with("OverflowError"), "{e}");
    }

    #[test]
    fn to_list_refuses_what_it_cannot_read() {
        let ints: [i32; 1] = [1];
        assert!(borrowed_over(&ints, "int32", None, 0).read_list().unwrap_err().contains("no length"));
        assert!(borrowed_over(&ints, "void", Some(1), 0).read_list().unwrap_err().contains("mustbe"));
        assert!(borrowed_over(&ints, "Db", Some(1), 0).read_list().unwrap_err().contains("not supported yet"));
    }

    #[test]
    fn to_bytes_copies_character_data() {
        let text = *b"hi\0zz";
        assert_eq!(borrowed_over(&text, "uint8", Some(2), 0).read_bytes().unwrap(), b"hi");
        // 長さが無くても NUL 終端なら NUL の手前まで
        assert_eq!(borrowed_over(&text, "uint8", None, PTR_FLAG_NUL_TERMINATED).read_bytes().unwrap(), b"hi");
        let wide: [u16; 3] = [0x61, 0x3042, 0];
        assert_eq!(
            borrowed_over(&wide, "uint16", None, PTR_FLAG_NUL_TERMINATED).read_bytes().unwrap(),
            vec![0x61, 0x00, 0x42, 0x30]
        );
        assert!(borrowed_over(&text, "uint8", None, 0).read_bytes().is_err());
        let ints: [i32; 1] = [1];
        assert!(borrowed_over(&ints, "int32", Some(1), 0).read_bytes().unwrap_err().contains("character data"));
    }

    #[test]
    fn annotation_matches_the_original_element_type() {
        use super::super::Value;
        let p = Value::Ptr(Arc::new(PtrHandle::new(1, "Db", 0, None, PtrOwner::Borrowed, None).unwrap()));
        assert_eq!(ptr_ann_matches("Ptr", &p), Some(true));
        assert_eq!(ptr_ann_matches("Ptr[void]", &p), Some(true));
        assert_eq!(ptr_ann_matches("Ptr[Db]", &p), Some(true));
        assert_eq!(ptr_ann_matches("Ptr[m.Db]", &p), Some(true));
        assert_eq!(ptr_ann_matches("Ptr[File]", &p), Some(false));
        assert_eq!(ptr_ann_matches("Ptr[int32]", &Value::Int(1)), Some(false));
        assert_eq!(ptr_ann_matches("list[int]", &p), None);
    }

    /// 共有ヘッダ・C# の写しの flags の値がこちらと同じ（4-1）。
    #[test]
    fn protocol_files_agree() {
        let header = include_str!("../../../bridge/ptr_protocol/ar_ptr_protocol.h");
        let cs = include_str!("../../../bridge/ptr_protocol/ArPtrProtocol.cs");
        let c_define = |name: &str| -> u32 {
            let line = header.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
            line.split_whitespace().nth(2).unwrap().trim_end_matches('u').parse().unwrap()
        };
        let cs_const = |name: &str| -> u32 {
            let key = format!("public const uint {name} = ");
            let line = cs.lines().find(|l| l.contains(&key)).expect(name);
            line.split(&key).nth(1).unwrap().trim_end_matches(';').trim().parse().unwrap()
        };
        assert_eq!(c_define("AR_PTR_CONST"), PTR_FLAG_CONST);
        assert_eq!(c_define("AR_PTR_NUL_TERMINATED"), PTR_FLAG_NUL_TERMINATED);
        assert_eq!(c_define("AR_PTR_STATIC"), PTR_FLAG_STATIC);
        assert_eq!(cs_const("Const"), PTR_FLAG_CONST);
        assert_eq!(cs_const("NulTerminated"), PTR_FLAG_NUL_TERMINATED);
        assert_eq!(cs_const("Static"), PTR_FLAG_STATIC);
        // 欄の並び（名前の順）も同じ
        let order = |text: &str, names: &[&str]| -> Vec<usize> { names.iter().map(|n| text.find(n).expect(n)).collect() };
        let c = order(header, &["uint64_t     address;", "elem_type;", "flags;", "_reserved;", "length;", "release;", "release_ctx;"]);
        let k = order(cs, &["ulong Address;", "ElemType;", "uint Flags;", "uint Reserved;", "long Length;", "> Release;", "ReleaseCtx;"]);
        assert!(c.windows(2).all(|w| w[0] < w[1]), "header field order");
        assert!(k.windows(2).all(|w| w[0] < w[1]), "C# field order");
    }

    /// 写しの並びが C のヘッダと同じ（64 ビットで 48 バイト・欄の位置）。
    #[test]
    fn protocol_layout_matches_the_header() {
        use std::mem::{offset_of, size_of};
        assert_eq!(size_of::<PtrProtocolRaw>(), 48);
        assert_eq!(offset_of!(PtrProtocolRaw, address), 0);
        assert_eq!(offset_of!(PtrProtocolRaw, elem_type), 8);
        assert_eq!(offset_of!(PtrProtocolRaw, flags), 16);
        assert_eq!(offset_of!(PtrProtocolRaw, _reserved), 20);
        assert_eq!(offset_of!(PtrProtocolRaw, length), 24);
        assert_eq!(offset_of!(PtrProtocolRaw, release), 32);
        assert_eq!(offset_of!(PtrProtocolRaw, release_ctx), 40);
    }

    #[test]
    fn keeps_the_library_alive_while_any_copy_remains() {
        let lib: PtrKeepAlive = Arc::new(String::from("stand-in for a loaded DLL"));
        let weak = Arc::downgrade(&lib);
        let p = Arc::new(PtrHandle::new(0x1000, "void", 0, None, PtrOwner::Borrowed, Some(lib)).unwrap());
        let copy = Arc::clone(&p);
        drop(p);
        assert!(weak.upgrade().is_some(), "a copy still holds the library");
        drop(copy);
        assert!(weak.upgrade().is_none(), "the last copy lets the library go");
    }
}
