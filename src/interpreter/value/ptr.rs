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

/// `PtrProtocol` の flags（D-1・D-15）。⚠ 4-1 の共有ヘッダ（`ar_ptr_protocol.h`）と同じ値にすること。
pub const PTR_FLAG_CONST: u32 = 1;
/// 文字データが NUL 終端である（`length` は NUL を含まない）。
pub const PTR_FLAG_NUL_TERMINATED: u32 = 2;
/// DLL のアンロードまで有効で中身も変わらない。`release` が NULL でよいのはこのときだけ（D-15）。
pub const PTR_FLAG_STATIC: u32 = 4;

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
