# PtrProtocol — 外部の関数とポインタを受け渡す約束

外部の関数（C / C++・C# の DLL・Rust の crate）と、**配列・文字データ・所有権のあるメモリ**を受け渡すときの約束です。
生のポインタは長さも要素の型も解放の責任も持たないので、Arrow は推測しません（`implementation_plans/ffi_value_exchange_plan.md`
D-1・D-7）。長さ・型・解放の責任はこの構造体にだけ書きます。

| 言語 | 定義 |
|---|---|
| C / C++ | [`bridge/ptr_protocol/ar_ptr_protocol.h`](../bridge/ptr_protocol/ar_ptr_protocol.h)（`ArPtrProtocol`） |
| C# | [`bridge/ptr_protocol/ArPtrProtocol.cs`](../bridge/ptr_protocol/ArPtrProtocol.cs)（`[StructLayout(Sequential)]`） |
| Arrow の実装 | `src/interpreter/value/ptr.rs`（`PtrProtocolRaw`） |

⚠ 3 つは同じ並び・同じ値です（64 ビットで 48 バイト）。単体テストが突き合わせています。

## 構造体

```c
typedef struct ArPtrProtocol {
    uint64_t     address;     /* 先頭のアドレス（NULL は 0） */
    const char*  elem_type;   /* 要素の型名（静的な文字列） */
    uint32_t     flags;       /* AR_PTR_* の組み合わせ */
    uint32_t     _reserved;   /* 0 */
    int64_t      length;      /* 要素の数（単体は 1。NUL 終端で数えない場合は -1） */
    ArPtrRelease release;     /* 解放関数 void (*)(void* ctx, uint64_t address)。NULL は STATIC のときだけ */
    void*        release_ctx; /* 解放に要る情報 */
} ArPtrProtocol;
```

## 要素の型名（`elem_type`）

Arrow 側では `Ptr[T]` の `T` になります（`docs/grammar/08_type_system.md` の「`Ptr[T]`」）。知らない名前はエラーです。

| `elem_type` | Arrow | 展開 |
|---|---|---|
| `"int8"` `"int16"` `"int32"` `"int64"` `"uint8"` `"uint16"` `"uint32"` `"uint64"` | `Ptr[int32]` など | `to_list()` → `list[int]`（`uint64` の 2^63 以上は OverflowError） |
| `"float32"` `"float64"` | `Ptr[float64]` など | `to_list()` → `list[float]` |
| `"uint8"`（`char` のバイト列）・`"uint16"`（`wchar_t` / `char16_t` の UTF-16） | 同上 | `to_bytes()` → `bytes`（文字列にするには `decode(encoding)`） |
| ヘッダの構造体の名前（`"Point"`） | `Ptr[Point]` | `to_list()` → `list[Point]` |
| `"void"` | `Ptr[void]` | 展開できない（`p mustbe Ptr[T]` で戻す） |

- `bool` は使いません（C# の `UnmanagedCallersOnly` に載らず、Arrow の C 互換レイアウトにもありません）。真偽値の配列は `uint8` にします
- Arrow の `int` / `float` に当たる名前はありません。C の幅で書きます

## flags

| 名前（C / C#） | 値 | 意味 |
|---|---|---|
| `AR_PTR_CONST` / `Const` | 1 | Arrow から書き換えない（読むだけのメモリ） |
| `AR_PTR_NUL_TERMINATED` / `NulTerminated` | 2 | 文字データが NUL で終わる。`length` が -1 なら NUL の手前までを読む |
| `AR_PTR_STATIC` / `Static` | 4 | DLL のアンロードまで有効で中身も変わらない。`release` は NULL でよい |

## 解放の約束

- **Arrow が受け取ったもの（戻り値）**: Arrow は最後の参照がなくなったとき（または `p.release()`）に
  `release(release_ctx, address)` を **1 回だけ**呼びます。それまで `address` の指すメモリは有効で、中身も変わらない
  ことが約束です（Arrow はいつでも `to_list()` / `to_bytes()` で読めます。期限切れの検査はしません）
- `release` が NULL でよいのは `AR_PTR_STATIC` のときだけです。STATIC の無い NULL は Arrow 側でエラーになります
- 次の呼び出しで上書きされる一時的なバッファを渡してはいけません。コピーして `release` を付けて渡します
  （違反は Arrow からは検知できません）
- **Arrow から渡したもの（引数）**: その呼び出しの間だけ有効です。保持したいなら外部の側でコピーします
- **外部の関数の側で解放するとき**: `ArPtrProtocol*` で受け取り、解放したら `address` を 0 に書き戻します。Arrow は
  呼び出しの後でそれを見て、その `Ptr` を使えなくします（以後の使用はエラー。`release` は呼びません）
- `Ptr` が残っている間、Arrow はその DLL をアンロードしません（`release` は DLL の中にあるため）

## 例（C）

```c
#include "ar_ptr_protocol.h"
#include <stdlib.h>

static void free_ints(void* ctx, uint64_t address) { (void)ctx; free((void*)(uintptr_t)address); }

ArPtrProtocol make_squares(int n) {
    int32_t* xs = (int32_t*)malloc(sizeof(int32_t) * n);
    for (int i = 0; i < n; i++) xs[i] = i * i;
    ArPtrProtocol p = { (uint64_t)(uintptr_t)xs, "int32", 0, 0, n, free_ints, NULL };
    return p;
}
```

```arrow
import[cpp-dll] squares as sq
let p = sq.make_squares(4)        # Option[Ptr[int32]]
match p:
    is None: print("NULL")
    is Ptr[int32]: print(p.to_list())   # [0, 1, 4, 9]
```

C/C++（`import[cpp-dll]` / `import[cpp-lib]`）では、ヘッダの `ArPtrProtocol` を認識し（4-2）、引数へ `Ptr` / `None` を渡し（4-4）、
値返しを `Option[Ptr[void]]` として受け取ります（4-5・`examples/interop/cpp_ptr_protocol.ar`）。Arrow の `list` / `bytes` を渡すと、
呼び出しの間だけ有効なコピーの `ArPtrProtocol` になります（4-7・`cpp_list_to_c.ar`。要素の型は int → `"int64"`・float →
`"float64"`・C の構造体 → 構造体名・bytes → `"uint8"`、解放関数は NULL）。⚠ C# / Rust の受け渡しはフェーズ 5 で入ります。
