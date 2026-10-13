// ArPtrProtocol.cs — Arrow とポインタを受け渡す約束 PtrProtocol の C# の写し（ffi_value_exchange_plan.md 4-1・D-1）
//
// C の ar_ptr_protocol.h と同じ並び（[StructLayout(Sequential)]・64 ビットで 48 バイト）。値・並びを変えるときは
// 3 つ（C のヘッダ・この写し・src/interpreter/value/ptr.rs）を同時に直すこと。仕様: docs/ptr_protocol.md
//
// ⚠ bool は使わない（UnmanagedCallersOnly の引数・戻り値に載らず、Arrow の C 互換レイアウトにも無い）。
// ⚠ csproj に <AllowUnsafeBlocks>true</AllowUnsafeBlocks> が要る（関数ポインタ・ポインタを使う）。

using System.Runtime.InteropServices;

namespace ArrowInterop;

[StructLayout(LayoutKind.Sequential)]
public unsafe struct ArPtrProtocol
{
    /// <summary>flags: Arrow から書き換えない（読むだけのメモリ）。</summary>
    public const uint Const = 1;
    /// <summary>flags: 文字データが NUL で終わる（Length は NUL を含まない数か、分からなければ -1）。</summary>
    public const uint NulTerminated = 2;
    /// <summary>flags: DLL のアンロードまで有効で中身も変わらない（Release は null でよい）。</summary>
    public const uint Static = 4;

    /// <summary>先頭のアドレス（NULL は 0）。</summary>
    public ulong Address;
    /// <summary>要素の型名（NUL 終端の UTF-8・静的な文字列。"int32" / "float64" / "uint8" / 構造体名 / "void"）。</summary>
    public byte* ElemType;
    /// <summary>Const / NulTerminated / Static の組み合わせ。</summary>
    public uint Flags;
    /// <summary>0 にする（C と並びを揃えるための明示の詰め物）。</summary>
    public uint Reserved;
    /// <summary>要素の数（単体は 1。NUL 終端で数えない場合は -1）。</summary>
    public long Length;
    /// <summary>解放関数 (ctx, address)。Arrow が使い終わったら 1 回だけ呼ぶ。null は Static のときだけ。</summary>
    public delegate* unmanaged<void*, ulong, void> Release;
    /// <summary>解放に要る情報（GCHandle を IntPtr にしたものなど）。</summary>
    public void* ReleaseCtx;
}
