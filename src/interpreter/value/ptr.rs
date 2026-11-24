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
    /// ⚠ 要素の型は作られたときの型（`elem_type`）で読む。数値だけ（構造体の要素は `classes/ptr_methods.rs` の `to_list` が
    ///   `read_struct_elements` で読む・4-6・7-14）。`void` は読めない、長さの分からない生のポインタ（D-7）も読めない。どれも誤り。
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

    /// 構造体の配列を `elem_size` バイトずつに分けてコピーする（`Ptr[構造体].to_list()`・4-6）。長さが要る。
    pub fn read_struct_elements(&self, elem_size: usize) -> Result<Vec<Vec<u8>>, String> {
        let len = self.known_length()?;
        let addr = self.address()?;
        // SAFETY: `PtrProtocol` が `length` 個の要素が並ぶと保証する（D-15）
        let all = unsafe { std::slice::from_raw_parts(addr as usize as *const u8, len * elem_size) };
        Ok(all.chunks_exact(elem_size.max(1)).map(|c| c.to_vec()).collect())
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

// ── C へ渡すときの規則（4-3・4-4）。ハンドル経路のコールバックと型つき経路（`TypedArgs::marshal`）が共有する唯一の定義 ──

/// `void*` / 構造体ポインタの引数に渡すアドレス。`Ptr`（要素型が `elem` と合うもの・`void` はどれでも）か `None`（0）。
/// 解放済み・C の側で解放済みの `Ptr` と、それ以外の値はエラー。
pub(crate) fn ptr_arg_address(v: &super::Value, elem: &str) -> Result<u64, String> {
    match v {
        super::Value::None => Ok(0),
        super::Value::Ptr(p) => {
            if ptr_ann_matches(&format!("Ptr[{elem}]"), v) != Some(true) {
                return Err(format!(
                    "TypeError: a `{elem}*` parameter needs a Ptr[{elem}] (or Ptr[void] for void*), not Ptr[{}]",
                    p.elem_type
                ));
            }
            p.address()
        }
        other => Err(format!(
            "TypeError: a `{elem}*` parameter needs a Ptr or None, not '{}'",
            crate::interpreter::ops::typecheck::runtime_type_name(other)
        )),
    }
}

/// `const char*` の引数に渡すバイト列（NUL を付けたコピー）。`bytes` か `None`（`None` を返す＝NULL）。
/// `str` は文字コードを書かせるためにエラー（D-13）、NUL を含む `bytes` は C の側で途中で切れるのでエラー。
pub(crate) fn c_string_arg(v: &super::Value) -> Result<Option<Vec<u8>>, String> {
    match v {
        super::Value::None => Ok(None),
        super::Value::Bytes(b) => {
            if b.contains(&0) {
                return Err(
                    "ValueError: bytes passed to a C string (`const char*`) parameter must not contain NUL (the C side would cut it there)"
                        .to_string(),
                );
            }
            let mut buf = b.to_vec();
            buf.push(0);
            Ok(Some(buf))
        }
        super::Value::Str(_) => Err(
            "TypeError: a C string (`const char*`) parameter needs bytes, not 'str'; encode it (e.g. s.encode(\"utf-8\"))"
                .to_string(),
        ),
        other => Err(format!(
            "TypeError: a C string (`const char*`) parameter needs bytes, not '{}'",
            crate::interpreter::ops::typecheck::runtime_type_name(other)
        )),
    }
}

/// `ArPtrProtocol` の引数に渡す構造体を `Ptr` から組み立てる（4-4）。`None` はアドレス 0 の構造体。
///
/// - 解放関数を渡すのは、C の側で解放できる `ArPtrProtocol*`（`writable`）のときだけ。値渡し・`const` ポインタでは
///   NULL にする（C が黙って解放すると、Arrow が後でもう一度解放してしまうため）
/// - 返す `PtrProtocolArg` は呼び出しが終わるまで生かしておく（構造体と要素の型名の文字列を指すため）。`writable` なら
///   呼び出しの後で `finish` を呼ぶ（C が `address` を 0 に書き戻していたら、その `Ptr` を使えなくする・D-3）
pub(crate) fn protocol_arg(v: &super::Value, writable: bool) -> Result<PtrProtocolArg, String> {
    // Arrow の `list` / `bytes` は呼び出しの間だけ有効なコピーとして渡す（4-7・D-15）
    if let super::Value::List(_) | super::Value::Bytes(_) = v {
        return copied_protocol_arg(v, writable, None);
    }
    let (raw, elem, ptr) = match v {
        super::Value::None => (
            PtrProtocolRaw {
                address: 0,
                elem_type: std::ptr::null(),
                flags: 0,
                _reserved: 0,
                length: 0,
                release: None,
                release_ctx: std::ptr::null_mut(),
            },
            None,
            None,
        ),
        super::Value::Ptr(p) => {
            let address = p.address()?;
            let elem = std::ffi::CString::new(p.elem_type.clone())
                .map_err(|_| format!("ValueError: Ptr element type '{}' contains NUL", p.elem_type))?;
            let (release, release_ctx) = match (writable, p.owner) {
                (true, PtrOwner::Owned { release, ctx }) => (Some(release), ctx as *mut std::ffi::c_void),
                _ => (None, std::ptr::null_mut()),
            };
            let raw = PtrProtocolRaw {
                address,
                elem_type: elem.as_ptr(),
                flags: p.flags,
                _reserved: 0,
                length: p.length.unwrap_or(-1),
                release,
                release_ctx,
            };
            (raw, Some(elem), writable.then(|| p.clone()))
        }
        other => {
            return Err(format!(
                "TypeError: an `ArPtrProtocol` parameter needs a Ptr or None, not '{}'",
                crate::interpreter::ops::typecheck::runtime_type_name(other)
            ))
        }
    };
    Ok(PtrProtocolArg { raw: Box::new(raw), _elem: elem, ptr, buf: Vec::new(), writeback: None })
}

/// Arrow の `list` / `bytes` を、呼び出しの間だけ有効なコピーの `ArPtrProtocol` にする（4-7・D-15）。
///
/// 要素の型は中身で決める: `int` → `int64`、`float` → `float64`、正確なレイアウトの構造体のインスタンス → 構造体名
/// （C の sizeof ずつ並べる）、`bytes` → `uint8`。空の `list`・種類の混ざった `list`・それ以外の要素は型が決まらないので誤り。
/// 解放関数は NULL・STATIC は付けない（C が持ち続けてはいけない）。`writable`（`ArPtrProtocol*`）なら呼び出しの後で、C が
/// 書いた中身を `list` へ書き戻す（D-14。`bytes` は書き換えられないので渡せない）。
/// [`protocol_arg`] の、宣言から要素の型が分かる呼び出し用（cs-dll の配列の引数・5-5）。空の list は宣言の要素型で長さ 0 にする
/// （[`protocol_arg`] は要素の型が決まらないので TypeError）。
pub(crate) fn protocol_arg_declared(v: &super::Value, writable: bool, elem: &str) -> Result<PtrProtocolArg, String> {
    match v {
        super::Value::List(items) if items.borrow().is_empty() => copied_protocol_arg(v, writable, Some(elem)),
        _ => protocol_arg(v, writable),
    }
}

fn copied_protocol_arg(v: &super::Value, writable: bool, empty_elem: Option<&str>) -> Result<PtrProtocolArg, String> {
    use super::Value;
    let (elem, data, count, writeback): (String, Vec<u8>, usize, Option<ListWriteBack>) = match v {
        Value::Bytes(b) => {
            if writable {
                return Err(
                    "TypeError: bytes cannot receive what the library writes into an `ArPtrProtocol*`; pass a mut list[int]"
                        .to_string(),
                );
            }
            ("uint8".to_string(), b.to_vec(), b.len(), None)
        }
        Value::List(items_rc) => {
            let items = items_rc.borrow();
            if let (true, Some(elem)) = (items.is_empty(), empty_elem) {
                return finish_copied(elem.to_string(), Vec::new(), 0, None, writable);
            }
            if items.is_empty() {
                return Err("TypeError: an empty list cannot be passed as `ArPtrProtocol` (its element type is unknown)".to_string());
            }
            let kind = match &items[0] {
                Value::Int(_) => ListKind::Int,
                Value::Float(_) => ListKind::Float,
                Value::Instance(inst) => {
                    let class = inst.borrow().class.clone();
                    match &class.raw_layout {
                        Some(l) => ListKind::Struct(class.clone(), l.c_size()),
                        None => {
                            return Err(format!(
                                "TypeError: a list of '{}' cannot be passed as `ArPtrProtocol` (it needs a struct with an exact C layout)",
                                class.name
                            ))
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "TypeError: a list of '{}' cannot be passed as `ArPtrProtocol` (int, float or a C struct)",
                        crate::interpreter::ops::typecheck::runtime_type_name(other)
                    ))
                }
            };
            let mut data = Vec::new();
            for (i, x) in items.iter().enumerate() {
                match (&kind, x) {
                    (ListKind::Int, Value::Int(n)) => data.extend_from_slice(&n.to_le_bytes()),
                    (ListKind::Float, Value::Float(f)) => data.extend_from_slice(&f.to_le_bytes()),
                    (ListKind::Struct(class, size), Value::Instance(inst)) if std::rc::Rc::ptr_eq(&inst.borrow().class, class) => {
                        data.extend_from_slice(&inst.borrow().raw_bytes()[..*size]);
                    }
                    _ => {
                        return Err(format!(
                            "TypeError: element {i} of the list does not match the first element ('{}'); an `ArPtrProtocol` holds one element type",
                            crate::interpreter::ops::typecheck::runtime_type_name(x)
                        ))
                    }
                }
            }
            let elem = match &kind {
                ListKind::Int => "int64".to_string(),
                ListKind::Float => "float64".to_string(),
                ListKind::Struct(class, _) => class.name.clone(),
            };
            let n = items.len();
            drop(items);
            (elem, data, n, writable.then(|| ListWriteBack { list: items_rc.clone(), kind }))
        }
        _ => unreachable!("copied_protocol_arg takes a list or bytes"),
    };
    finish_copied(elem, data, count, writeback, writable)
}

/// コピーしたバイト列を `ArPtrProtocol` にする（[`copied_protocol_arg`] の後半）。
fn finish_copied(
    elem: String,
    data: Vec<u8>,
    count: usize,
    writeback: Option<ListWriteBack>,
    writable: bool,
) -> Result<PtrProtocolArg, String> {
    // 8 バイト境界に置く（C が int64 / double の配列として読む）
    let mut buf = vec![0u64; data.len().div_ceil(8).max(1)];
    // SAFETY: `buf` は `data.len()` バイト以上ある
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf.as_mut_ptr() as *mut u8, data.len()) };
    let elem_c = std::ffi::CString::new(elem).map_err(|_| "ValueError: element type contains NUL".to_string())?;
    let raw = PtrProtocolRaw {
        address: buf.as_ptr() as u64,
        elem_type: elem_c.as_ptr(),
        flags: if writable { 0 } else { PTR_FLAG_CONST },
        _reserved: 0,
        length: count as i64,
        release: None,
        release_ctx: std::ptr::null_mut(),
    };
    Ok(PtrProtocolArg { raw: Box::new(raw), _elem: Some(elem_c), ptr: None, buf, writeback })
}

/// `list` の要素の種類（`ArPtrProtocol` へのコピーと書き戻し・4-7）。
enum ListKind {
    Int,
    Float,
    /// 構造体のクラスと C の sizeof
    Struct(std::rc::Rc<crate::interpreter::ClassValue>, usize),
}

/// 呼び出しの後で `list` へ書き戻すもの（`ArPtrProtocol*`・4-7・D-14）。
struct ListWriteBack {
    list: std::rc::Rc<std::cell::RefCell<Vec<super::Value>>>,
    kind: ListKind,
}

/// [`protocol_arg`] が組み立てた構造体（呼び出しの間だけ生きる）。
pub struct PtrProtocolArg {
    /// C へ渡す構造体（`Box` なのでアドレスが動かない）。
    pub raw: Box<PtrProtocolRaw>,
    /// `raw.elem_type` が指す文字列。
    _elem: Option<std::ffi::CString>,
    /// C の側で解放できる（`ArPtrProtocol*`）ときの元の `Ptr`。
    ptr: Option<Arc<PtrHandle>>,
    /// `list` / `bytes` のコピー（`raw.address` が指す・4-7）。
    buf: Vec<u64>,
    /// 呼び出しの後で `list` へ書き戻すもの（`ArPtrProtocol*` に `list` を渡したとき・4-7）。
    writeback: Option<ListWriteBack>,
}

impl PtrProtocolArg {
    /// 構造体のアドレス（`ArPtrProtocol*` の引数、値渡しなら中継コードがここからコピーする）。
    pub fn address(&self) -> u64 {
        &*self.raw as *const PtrProtocolRaw as u64
    }

    /// 呼び出しの後で呼ぶ。C の側で解放されて `address` が 0 に書き戻されていたら、その `Ptr` を使えなくする。
    /// `list` を `ArPtrProtocol*` で渡していたら、C が書いた中身を `list` の要素へ書き戻す（4-7・D-14。要素の数は変えない）。
    pub fn finish(self) {
        if let Some(p) = &self.ptr {
            if self.raw.address == 0 {
                p.invalidate();
            }
        }
        if let Some(wb) = &self.writeback {
            // ⚠ C が `address` を書き換えた（解放したつもり・別の領域を指した）ら、中身は読まない
            if self.raw.address != self.buf.as_ptr() as u64 {
                return;
            }
            let bytes = unsafe { std::slice::from_raw_parts(self.buf.as_ptr() as *const u8, self.buf.len() * 8) };
            let mut items = wb.list.borrow_mut();
            for (i, item) in items.iter_mut().enumerate() {
                match &wb.kind {
                    ListKind::Int => {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
                        *item = super::Value::Int(i64::from_le_bytes(b));
                    }
                    ListKind::Float => {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&bytes[i * 8..i * 8 + 8]);
                        *item = super::Value::Float(f64::from_le_bytes(b));
                    }
                    ListKind::Struct(_, size) => {
                        if let super::Value::Instance(inst) = item {
                            inst.borrow_mut().raw_bytes_mut()[..*size].copy_from_slice(&bytes[i * size..(i + 1) * size]);
                        }
                    }
                }
            }
        }
    }

    /// C が `ArPtrProtocol*` の `address` に書いた値を受け取る（ハンドル経路は中継コードの局所変数に書かれる・4-7）。
    pub fn set_address_after(&mut self, address: u64) {
        self.raw.address = address;
    }
}

impl std::fmt::Debug for PtrProtocolArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PtrProtocolArg {{ address: {:#x} }}", self.raw.address)
    }
}

/// 型つき経路の戻り値（`char*` / `wchar_t*`）を `bytes` へ（NUL までをコピーする・`wchar_t` は UTF-16LE）。NULL は `None`。
/// `wchar_t` の幅は `ar_config.json` の `"os"`（7-32。Linux / macOS の UTF-32 は UTF-16LE に符号化し直す）。
///
/// # Safety
/// `address` は NUL 終端の文字列を指すか 0 であること（C の関数の戻り値）。
pub(crate) unsafe fn c_string_ret(address: u64, wide: bool) -> Result<super::Value, String> {
    if address == 0 {
        return Ok(super::Value::None);
    }
    Ok(if wide {
        let unit = crate::cpp_header::target_os().wchar_bytes();
        super::Value::bytes(crate::cpp_header::target::wide_text_utf16le(address as usize as *const u8, unit)?)
    } else {
        super::Value::bytes(std::ffi::CStr::from_ptr(address as usize as *const std::ffi::c_char).to_bytes().to_vec())
    })
}

/// 型つき経路の戻り値（`void*` / 構造体・数値のポインタ）を借用の `Ptr[elem]` へ（Arrow は解放しない・D-7）。NULL は `None`。
pub(crate) fn raw_pointer_ret(address: u64, elem: &str) -> Result<super::Value, String> {
    if address == 0 {
        return Ok(super::Value::None);
    }
    let h = PtrHandle::new(address, elem, 0, None, PtrOwner::Borrowed, None)?;
    Ok(super::Value::Ptr(Arc::new(h)))
}

/// 外部の関数が返した `ArPtrProtocol` を `Ptr` にする（4-5・D-10・D-12・D-15）。アドレス 0（NULL）は `None`。
///
/// - 要素の型名は数値（`int8` … `float64`）・`void`・そのヘッダが宣言した構造体（`known_structs`）だけ。知らない名前は誤り
/// - `length` は 0 以上、または NUL 終端（`AR_PTR_NUL_TERMINATED`）のときだけ -1（数えない）
/// - 解放関数があれば所有（最後の参照で 1 回だけ呼ぶ）。NULL でよいのは `AR_PTR_STATIC` のときだけ
/// ⚠ 誤りのとき、メモリは解放しない（解放関数を信じられない形で届いたため。漏れても落ちない側に倒す）。
pub(crate) fn ptr_from_protocol(raw: &PtrProtocolRaw, known_structs: &[Arc<str>]) -> Result<super::Value, String> {
    if raw.address == 0 {
        return Ok(super::Value::None);
    }
    if raw.elem_type.is_null() {
        return Err("ValueError: an ArPtrProtocol from the library has no element type (elem_type is NULL)".to_string());
    }
    // SAFETY: `elem_type` は静的な NUL 終端の文字列という約束（D-12・`ar_ptr_protocol.h`）
    let elem = unsafe { std::ffi::CStr::from_ptr(raw.elem_type) }
        .to_str()
        .map_err(|_| "ValueError: the element type of an ArPtrProtocol is not valid UTF-8".to_string())?;
    if num_width(elem).is_none() && elem != "void" && !known_structs.iter().any(|s| &**s == elem) {
        return Err(format!(
            "ValueError: unknown element type '{elem}' in an ArPtrProtocol (expected int8 … int64, uint8 … uint64, float32, float64, void, or a struct declared in the header)"
        ));
    }
    let length = match raw.length {
        n if n >= 0 => Some(n),
        -1 if raw.flags & PTR_FLAG_NUL_TERMINATED != 0 => None,
        n => {
            return Err(format!(
                "ValueError: an ArPtrProtocol has length {n}; -1 is allowed only with AR_PTR_NUL_TERMINATED"
            ))
        }
    };
    let owner = match raw.release {
        Some(release) => PtrOwner::Owned { release, ctx: raw.release_ctx as usize },
        None if raw.flags & PTR_FLAG_STATIC != 0 => PtrOwner::Static,
        None => {
            return Err(format!(
                "ValueError: an ArPtrProtocol (Ptr[{elem}]) has no release function and is not AR_PTR_STATIC; the library must say who frees it"
            ))
        }
    };
    let h = PtrHandle::new(raw.address, elem, raw.flags, length, owner, None)?;
    Ok(super::Value::Ptr(Arc::new(h)))
}

/// ポインタのフィールドを持つ C の構造体の並び（`Ptr[構造体].to_list()`・ffi_value_exchange_plan.md 7-14）。
///
/// 数値だけの構造体は `RawLayout`（C のバイト列をインスタンスへ写す・4-6）を使う。こちらはフィールドごとに Arrow の値へ
/// 変換する構造体のためのもので、cpp のヘッダから実行時に作る（`CStructDef::c_struct_layout`）。並びは C の規則: 各フィールドを
/// 自分の幅の境界へ揃え、全体の大きさをいちばん大きい境界の倍数へ切り上げる（ポインタはプロセスのポインタの幅）。
#[derive(Debug, Clone)]
pub struct CStructLayout {
    /// 宣言順の（構造体の先頭からのバイト位置, 種類）。
    pub fields: Vec<(usize, CField)>,
    /// C の `sizeof`（配列の要素の間隔）。
    pub size: usize,
}

/// C の構造体のフィールドの種類と、Arrow の値への変換（D-7。値返しの構造体の中継コード `cpp_bridge::codegen::field_to_handle`
/// と同じ規則にすること）。
#[derive(Debug, Clone, PartialEq)]
pub enum CField {
    /// 数値（C の幅つきの名前 `int32` / `float64` …・`read_num` で読む。`uint64` の 2^63 以上は OverflowError）。
    Num(String),
    /// C の `bool`（1 バイト。0 以外は真）。
    Bool,
    /// `char*` / `wchar_t*`: NUL 終端の文字データの `bytes`（`unit` は 1 単位のバイト数: `char` は 1、`wchar_t` は OS で 2 か 4。
    /// `wchar_t` はどの OS でも UTF-16LE の bytes にする）。NULL は `None`。
    Text { unit: usize },
    /// ほかのポインタ（`void*`・構造体・数値へのポインタ）: 借用の `Ptr[elem]`（Arrow は解放しない）。NULL は `None`。
    Ptr(String),
}

/// C の構造体 1 つのバイト列（`layout.size` バイト）から、宣言順のフィールドの値を作る（7-14）。
///
/// ⚠ 文字データのフィールドは、指す先の NUL までをコピーする（D-2: 展開した値は C のメモリとつながらない）。
///   ポインタのフィールドは借用の `Ptr`（生のヘッダの戻り値と同じ・D-7）。
///
/// # Safety
/// `bytes` は C の構造体の写しで、そのポインタのフィールドは NULL か、読める NUL 終端の文字データ・メモリを指すこと
/// （`PtrProtocol` の規約・D-15）。
pub unsafe fn read_c_struct_fields(bytes: &[u8], layout: &CStructLayout) -> Result<Vec<super::Value>, String> {
    use super::Value;
    if bytes.len() < layout.size {
        return Err(format!(
            "RuntimeError: a C struct needs {} bytes, but {} were given",
            layout.size,
            bytes.len()
        ));
    }
    let addr_at = |off: usize| -> usize {
        let mut b = [0u8; std::mem::size_of::<usize>()];
        b.copy_from_slice(&bytes[off..off + std::mem::size_of::<usize>()]);
        usize::from_le_bytes(b)
    };
    layout
        .fields
        .iter()
        .map(|(off, field)| {
            Ok(match field {
                CField::Num(elem) => read_num(elem, bytes[*off..].as_ptr())?,
                CField::Bool => Value::Bool(bytes[*off] != 0),
                CField::Text { unit } => match addr_at(*off) {
                    0 => Value::None,
                    // `char*` は NUL の手前まで
                    p if *unit == 1 => Value::bytes(std::ffi::CStr::from_ptr(p as *const std::ffi::c_char).to_bytes().to_vec()),
                    // `wchar_t*` はどの OS でも UTF-16LE（D-9。UTF-32 の OS は符号化し直す・7-32）
                    p => Value::bytes(crate::cpp_header::target::wide_text_utf16le(p as *const u8, *unit)?),
                },
                CField::Ptr(elem) => match addr_at(*off) {
                    0 => Value::None,
                    p => Value::Ptr(Arc::new(PtrHandle::new(p as u64, elem.clone(), 0, None, PtrOwner::Borrowed, None)?)),
                },
            })
        })
        .collect()
}

/// 数値の要素の名前か（`int8` … `float64`）。
pub(crate) fn is_num_elem(elem: &str) -> bool {
    num_width(elem).is_some()
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
mod tests;
