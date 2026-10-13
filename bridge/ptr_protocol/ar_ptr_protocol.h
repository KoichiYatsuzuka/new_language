/*
 * ar_ptr_protocol.h - how C/C++ passes pointers to and from Arrow (PtrProtocol).
 * (implementation_plans/ffi_value_exchange_plan.md, tasks 4-1 / D-1)
 *
 * Pass arrays, character data and owned memory as this struct instead of a raw pointer.
 * The length, the element type and who releases the memory are written here and nowhere else
 * (Arrow does not read SAL annotations or other side declarations).
 * Spec: docs/ptr_protocol.md. The C# mirror is ArPtrProtocol.cs; keep the three in sync.
 *
 * NOTE: keep this file ASCII-only. MSVC reads sources as CP932 by default, and a UTF-8
 * comment can swallow the next line of a header that includes this one.
 *
 *   ArPtrProtocol p;
 *   p.address     = (uint64_t)(uintptr_t)data;   // first element (NULL is 0)
 *   p.elem_type   = "int32";                     // element type name (a static string, table below)
 *   p.flags       = 0;                           // AR_PTR_CONST, ...
 *   p._reserved   = 0;
 *   p.length      = n;                           // number of elements (1 for a single value)
 *   p.release     = my_free;                     // Arrow calls it exactly once when done
 *   p.release_ctx = NULL;                        // passed back to release
 *
 * Element type names (elem_type) -> Arrow type:
 *   "int8" "int16" "int32" "int64" "uint8" "uint16" "uint32" "uint64" -> Ptr[int8] ... (to_list() gives list[int])
 *   "float32" "float64"                                               -> Ptr[float32] ... (to_list() gives list[float])
 *   "uint8" (char bytes), "uint16" (wchar_t / char16_t UTF-16)         -> to_bytes() gives bytes
 *   a struct name from the header ("Point")                           -> Ptr[Point]
 *   "void"                                                            -> Ptr[void] (cannot be expanded)
 *   An unknown name is an error on the Arrow side. Do not use bool (it does not fit C#'s
 *   UnmanagedCallersOnly nor Arrow's C-compatible layout); use uint8 for arrays of flags.
 *
 * Releasing:
 *   - Arrow calls release(release_ctx, address) exactly once, when the last reference goes away
 *     (or on p.release()). Until then the memory must stay valid and unchanged: Arrow may read it
 *     with to_list() at any time.
 *   - release may be NULL only with AR_PTR_STATIC (valid and unchanged until the DLL is unloaded).
 *     A NULL release without STATIC is an error on the Arrow side.
 *   - Never pass a temporary buffer that the next call overwrites; copy it and attach a release.
 *   - What Arrow passes to C (arguments) is valid only during the call; copy it to keep it.
 *   - To release on the C side, take ArPtrProtocol* and write address = 0 after freeing.
 *     Arrow checks it after the call and makes that Ptr unusable (release is not called).
 */
#ifndef AR_PTR_PROTOCOL_H
#define AR_PTR_PROTOCOL_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* flags (bit set). Same values as PTR_FLAG_* in src/interpreter/value/ptr.rs */
#define AR_PTR_CONST          1u  /* Arrow does not write to it (read-only memory) */
#define AR_PTR_NUL_TERMINATED 2u  /* character data ends with NUL (length excludes it, or -1 if unknown) */
#define AR_PTR_STATIC         4u  /* valid and unchanged until the DLL is unloaded (release may be NULL) */

/* Release function. ctx is release_ctx, address is the address that was passed. */
typedef void (*ArPtrRelease)(void* ctx, uint64_t address);

typedef struct ArPtrProtocol {
    uint64_t     address;     /* first element (NULL is 0) */
    const char*  elem_type;   /* element type name (a static string, table above) */
    uint32_t     flags;       /* AR_PTR_* */
    uint32_t     _reserved;   /* 0 (explicit padding so that C# lays it out the same way) */
    int64_t      length;      /* number of elements (1 for a single value; -1 = count up to NUL) */
    ArPtrRelease release;     /* release function (NULL only with AR_PTR_STATIC) */
    void*        release_ctx; /* what release needs (heap, GCHandle, the owning vector, ...) */
} ArPtrProtocol;

/* 48 bytes on 64-bit targets. Arrow (Rust) and the C# mirror rely on the same layout. */
typedef char ar_ptr_protocol_size_check[(sizeof(void*) != 8 || sizeof(ArPtrProtocol) == 48) ? 1 : -1];

#ifdef __cplusplus
}
#endif

#endif /* AR_PTR_PROTOCOL_H */
