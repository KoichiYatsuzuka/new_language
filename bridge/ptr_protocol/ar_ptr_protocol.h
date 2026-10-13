/*
 * ar_ptr_protocol.h — Arrow とポインタを受け渡す約束（PtrProtocol・ffi_value_exchange_plan.md 4-1・D-1）
 *
 * 配列・文字データ・所有権のあるメモリを Arrow と受け渡すときは、生のポインタではなくこの構造体で渡す。
 * 長さ・要素の型・解放の責任を、ここにだけ書く（D-7。SAL 注釈や補足の宣言は読まない）。
 * 仕様: docs/ptr_protocol.md（C# の写しは ArPtrProtocol.cs。値は 3 つで同じにすること）
 *
 *   ArPtrProtocol p;
 *   p.address     = (uint64_t)(uintptr_t)data;   // 先頭のアドレス（NULL は 0）
 *   p.elem_type   = "int32";                     // 要素の型名（静的な文字列・下の表）
 *   p.flags       = 0;                           // AR_PTR_CONST など
 *   p._reserved   = 0;
 *   p.length      = n;                           // 要素の数（単体は 1）
 *   p.release     = my_free;                     // Arrow が使い終わったら 1 回だけ呼ぶ
 *   p.release_ctx = NULL;                        // release に渡す情報
 *
 * 要素の型名（elem_type）→ Arrow の型:
 *   "int8" "int16" "int32" "int64" "uint8" "uint16" "uint32" "uint64"  → Ptr[int8] … （to_list() は list[int]）
 *   "float32" "float64"                                                 → Ptr[float32] …（to_list() は list[float]）
 *   "uint8"（char のバイト列）・"uint16"（wchar_t / char16_t の UTF-16）  → to_bytes() で bytes にできる
 *   ヘッダの構造体の名前（"Point" など）                                  → Ptr[Point]
 *   "void"                                                              → Ptr[void]（展開できない）
 *   ⚠ 知らない名前は Arrow 側でエラーになる（D-4）。bool は使わない（C# の UnmanagedCallersOnly と
 *     Arrow の C 互換レイアウトに載らない）。真偽値の配列は uint8 にする。
 *
 * 解放の約束（D-15）:
 *   - Arrow は最後の参照がなくなったとき（または p.release()）に release(release_ctx, address) を 1 回だけ呼ぶ。
 *     それまで address の指すメモリは有効で、中身も変わらないこと（Arrow はいつでも to_list() で読む）
 *   - release が NULL でよいのは flags に AR_PTR_STATIC があるときだけ（DLL のアンロードまで有効で中身も
 *     変わらないメモリ）。STATIC の無い NULL は Arrow 側でエラー
 *   - 次の呼び出しで上書きされる一時的なバッファを渡してはいけない（コピーして release を付けて渡す）
 *   - Arrow から C へ渡したもの（引数）は、その呼び出しの間だけ有効。保持したいなら C の側でコピーする
 *   - C の関数の側で解放するときは ArPtrProtocol* で受け取り、解放したら address を 0 に書き戻す。
 *     Arrow は呼び出しの後でそれを見て、その Ptr を使えなくする
 */
#ifndef AR_PTR_PROTOCOL_H
#define AR_PTR_PROTOCOL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* flags（ビットの組み合わせ）。⚠ src/interpreter/value/ptr.rs の PTR_FLAG_* と同じ値 */
#define AR_PTR_CONST          1u  /* Arrow から書き換えない（読むだけのメモリ） */
#define AR_PTR_NUL_TERMINATED 2u  /* 文字データが NUL で終わる（length は NUL を含まない数か、分からなければ -1） */
#define AR_PTR_STATIC         4u  /* DLL のアンロードまで有効で中身も変わらない（release は NULL でよい） */

/* 解放関数。ctx は release_ctx、address は渡したときの address */
typedef void (*ArPtrRelease)(void* ctx, uint64_t address);

typedef struct ArPtrProtocol {
    uint64_t     address;     /* 先頭のアドレス（NULL は 0） */
    const char*  elem_type;   /* 要素の型名（静的な文字列・上の表） */
    uint32_t     flags;       /* AR_PTR_* の組み合わせ */
    uint32_t     _reserved;   /* 0 にする（並びを C# と揃えるための明示の詰め物） */
    int64_t      length;      /* 要素の数（単体は 1。NUL 終端で数えない場合は -1） */
    ArPtrRelease release;     /* 解放関数（NULL は AR_PTR_STATIC のときだけ） */
    void*        release_ctx; /* 解放に要る情報（ヒープ・GCHandle・vector の本体など） */
} ArPtrProtocol;

/* 64 ビットの環境で 48 バイト。Arrow（Rust）と C# の写しが同じ並びであることを前提にしている */
typedef char ar_ptr_protocol_size_check[(sizeof(void*) != 8 || sizeof(ArPtrProtocol) == 48) ? 1 : -1];

#ifdef __cplusplus
}
#endif

#endif /* AR_PTR_PROTOCOL_H */
