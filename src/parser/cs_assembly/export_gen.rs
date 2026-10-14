// cs_assembly/export_gen.rs — cs-dll の export（`[UnmanagedCallersOnly]` の中継関数）を C# で生成する:
// generate_cs_exports（ffi_value_exchange_plan.md 5-1・`arrow --gen-cs-exports X.dll`）。
//
// 以前は利用者が手で書いていた（`examples/interop/cs_interop_test/ArrowExports.cs` の形）。生成するものは
// `cs_dll_runtime.rs` の ABI と同じ:
//   - 名前: `{Class}_new_{引数の数}` / `{Class}_{name}`（static）/ `{Class}_inst_{name}`（インスタンス・先頭はハンドル）。
//     `name` は型スタブの名前（プロパティは `getX` / `setX`）。オーバーロードは先頭だけ（Arrow もスタブの先頭で呼ぶ）
//   - 値: 整数・bool → `long`、浮動小数 → `long` のビット列、文字列 → `(byte*, int)`（null は長さ -1）、
//     クラス → オブジェクト表のハンドル（null は 0）。文字列の戻り値は末尾の `(byte** out_ptr, int* out_len)`
//   - エラー: C# の例外と引数の誤り（整数の幅・死んだハンドル）を捕まえて `arrow_bridge_take_error` に預ける。
//     Arrow は呼び出しのたびに引き取って例外にする。⚠ `[UnmanagedCallersOnly]` から例外が漏れるとプロセスが落ちる
//     （手書きの export は捕まえていなかった）
//
// 型スタブと同じ一覧（`assembly_model`）を読むので、Arrow が呼ぶ名前と型の export だけが並ぶ。スタブが未対応とした
// メンバー（コレクションなど）と、この生成器が扱えないもの（`object`・`ref` / `out`・ジェネリック・演算子）は
// 作らず、理由をコメントで残す（Arrow から呼ぶと「export が無い」エラーになる）。

use std::collections::{HashMap, HashSet};

use super::*;

/// C# の引数 1 つ分の受け取り方。
struct ParamAbi {
    /// export の引数の宣言（文字列は 2 つ）
    decls: Vec<String>,
    /// 受け取った値から C# の値を作る式
    expr: String,
}

/// 戻り値の返し方。
enum RetAbi {
    Void,
    /// `long` で返す。中身は C# の値（`$r`）から `long` を作る式
    Long(String),
    /// 文字列（`(byte** out_ptr, int* out_len)` へ書く）。中身は C# の値（`$r`）から `string?` を作る式
    Str(String),
}

/// 生成する C# のソース。`source` はメッセージ用の DLL の名前。
pub(crate) fn generate_cs_exports(pa: &ParsedAssembly, source: &str) -> String {
    let model = assembly_model(pa, CsBridge::Dll);
    // 型スタブにクラスとして出す、ジェネリックでない型（ハンドルで受け渡す）: スタブの名前 → C# の名前
    let classes: HashMap<String, String> = model
        .iter()
        .filter(|c| !c.is_interface && c.generic_param_names.is_empty())
        .map(|c| (c.name.clone(), format!("global::{}", c.cs_name)))
        .collect();

    let mut out = String::new();
    out.push_str(&header(source));
    for c in &model {
        out.push_str(&format!("\n    // ── {} ──\n", c.name));
        if c.is_interface {
            out.push_str("    // skipped: an interface (its objects are called through their own classes)\n");
            continue;
        }
        if !c.generic_param_names.is_empty() {
            out.push_str("    // skipped: a generic class (Arrow cannot give its type arguments)\n");
            continue;
        }
        let cs_class = format!("global::{}", c.cs_name);
        let mut seen: HashSet<String> = HashSet::new();
        for m in &c.members {
            let entry = match m.role {
                MemberRole::Ctor => format!("{}_new_{}", c.name, m.params.len()),
                _ if m.is_static => format!("{}_{}", c.name, m.arrow_name),
                _ => format!("{}_inst_{}", c.name, m.arrow_name),
            };
            let label = format!("{}.{}", c.name, m.arrow_name);
            if !seen.insert(entry.clone()) {
                out.push_str(&format!("    // skipped {label}: an overload (Arrow calls the first one with this name)\n"));
                continue;
            }
            match member_export(c, &cs_class, m, &entry, &classes) {
                Ok(code) => out.push_str(&code),
                Err(why) => out.push_str(&format!("    // skipped {label}: {why}\n")),
            }
        }
    }
    out.push_str("}\n");
    out
}

/// メンバー 1 つの export。作れなければ理由。
fn member_export(
    c: &CsClassModel,
    cs_class: &str,
    m: &CsMemberModel,
    entry: &str,
    classes: &HashMap<String, String>,
) -> Result<String, String> {
    if let Some(why) = &m.unsupported {
        return Err(format!("{why} (the type stub marks it unsupported)"));
    }
    if !m.generic_param_names.is_empty() {
        return Err("a generic method".to_string());
    }
    if matches!(m.role, MemberRole::Operator(_)) {
        return Err("an operator (Arrow does not call C# operators)".to_string());
    }
    if m.role == MemberRole::Ctor && c.is_abstract {
        return Err("a constructor of an abstract class".to_string());
    }
    if matches!(m.role, MemberRole::Setter(_)) && m.sig_len != 1 {
        return Err("an indexer setter".to_string());
    }
    if let Some(p) = m.params.iter().find(|p| p.byref) {
        return Err(format!("parameter `{}` is `ref` / `out` (not written back yet ⇒ 7-16)", p.name));
    }
    let what = format!("{}.{}", c.name, m.arrow_name);
    let mut decls: Vec<String> = Vec::new();
    let mut args: Vec<String> = Vec::new();
    if !m.is_static && m.role != MemberRole::Ctor {
        decls.push("long self".to_string());
    }
    for (i, p) in m.params.iter().enumerate() {
        let a = param_abi(&p.ty, p.ann.starts_with("Option["), i, classes)
            .map_err(|why| format!("parameter `{}` is {why}", p.name))?;
        decls.extend(a.decls);
        args.push(a.expr);
    }
    let ret = match m.role {
        MemberRole::Ctor | MemberRole::Setter(_) => RetAbi::Long(String::new()),
        _ => ret_abi(&m.ret, classes).map_err(|why| format!("it returns {why}"))?,
    };
    // C# の呼び出し式
    let target = if m.is_static { cs_class.to_string() } else { "obj".to_string() };
    let call = match &m.role {
        MemberRole::Ctor => format!("new {cs_class}({})", args.join(", ")),
        MemberRole::Getter(prop) if args.is_empty() => format!("{target}.{prop}"),
        MemberRole::Getter(_) => format!("{target}[{}]", args.join(", ")),
        MemberRole::Setter(prop) => format!("{target}.{prop} = {}", args.join(", ")),
        _ => format!("{target}.{}({})", m.cs_name, args.join(", ")),
    };
    let (ret_ty, body, on_error) = match (&m.role, &ret) {
        (MemberRole::Ctor, _) => ("long", format!("return ArrowObjects.Store({call});"), "return 0;"),
        (MemberRole::Setter(_), _) => ("void", format!("{call};"), "return;"),
        (_, RetAbi::Void) => ("void", format!("{};", await_task(&m.ret, &call)), "return;"),
        (_, RetAbi::Long(conv)) => {
            let conv = conv.replace("$r", "r");
            ("long", format!("var r = {};\n            return {conv};", await_task(&m.ret, &call)), "return 0;")
        }
        (_, RetAbi::Str(conv)) => {
            decls.push("byte** outPtr".to_string());
            decls.push("int* outLen".to_string());
            let conv = conv.replace("$r", "r");
            ("void", format!("var r = {};\n            ArrowAbi.WriteStr({conv}, outPtr, outLen);", await_task(&m.ret, &call)),
             "*outPtr = null; *outLen = -1; return;")
        }
    };
    let get_obj = if !m.is_static && m.role != MemberRole::Ctor {
        format!("var obj = ArrowObjects.Get<{cs_class}>(self, 0, what);\n            ")
    } else {
        String::new()
    };
    Ok(format!(
        "    [UnmanagedCallersOnly(EntryPoint = \"{entry}\")]\n    public static {ret_ty} {entry}({})\n    {{\n        const string what = \"{what}\";\n        try\n        {{\n            {get_obj}{body}\n        }}\n        catch (Exception e) {{ ArrowAbi.SetError(e); {on_error} }}\n    }}\n\n",
        decls.join(", ")
    ))
}

/// `Task<T>` / `ValueTask<T>` は待ってから返す（型スタブは `T`・ブリッジは同期で呼ぶ）。
fn await_task(ret: &CsTy, call: &str) -> String {
    match ret {
        CsTy::Generic { base, .. } if is_task(base) => format!("{call}.GetAwaiter().GetResult()"),
        _ => call.to_string(),
    }
}

fn is_task(base: &str) -> bool {
    matches!(base, "System.Threading.Tasks.Task" | "System.Threading.Tasks.ValueTask")
}

/// 引数の受け取り方（`i` は 0 始まり。メッセージは 1 始まり）。`nullable` は型スタブが `Option[T]` のとき。
fn param_abi(ty: &CsTy, nullable: bool, i: usize, classes: &HashMap<String, String>) -> Result<ParamAbi, String> {
    let n = i + 1;
    let a = format!("a{i}");
    let long = |expr: String| Ok(ParamAbi { decls: vec![format!("long {a}")], expr });
    match ty {
        CsTy::Bool => long(format!("{a} != 0")),
        CsTy::Int { bits: 64, signed: true } => long(a.clone()),
        CsTy::Int { bits, signed } => long(format!("ArrowAbi.{}({a}, {n}, what)", int_conv(*bits, *signed))),
        CsTy::NInt { signed: true } => long(format!("(nint){a}")),
        CsTy::NInt { signed: false } => long(format!("(nuint)ArrowAbi.ToUInt64({a}, {n}, what)")),
        CsTy::Float { bits: 32 } => long(format!("(float)BitConverter.Int64BitsToDouble({a})")),
        CsTy::Float { .. } => long(format!("BitConverter.Int64BitsToDouble({a})")),
        CsTy::Char => Ok(ParamAbi {
            decls: vec![format!("byte* {a}"), format!("int {a}n")],
            expr: format!("ArrowAbi.Char({a}, {a}n, {n}, what)"),
        }),
        CsTy::Str => Ok(ParamAbi {
            decls: vec![format!("byte* {a}"), format!("int {a}n")],
            expr: if nullable { format!("ArrowAbi.Str({a}, {a}n)") } else { format!("ArrowAbi.Str({a}, {a}n)!") },
        }),
        CsTy::Named { name, .. } => match classes.get(name) {
            Some(cs) if nullable => long(format!("{a} == 0 ? null : ArrowObjects.Get<{cs}>({a}, {n}, what)")),
            Some(cs) => long(format!("ArrowObjects.Get<{cs}>({a}, {n}, what)")),
            None => Err(format!("`{name}`, not a public class of this assembly")),
        },
        // `int?` などの値型の nullable（型スタブは `Option[int]`。ブリッジは null を渡さない）
        CsTy::Generic { base, args, .. } if base == "System.Nullable" && args.len() == 1 => {
            let inner = param_abi(&args[0], false, i, classes)?;
            Ok(ParamAbi { decls: inner.decls, expr: format!("({})({})", cs_value_type(&args[0])?, inner.expr) })
        }
        CsTy::Object => Err("`object` (the bridge lays it out by the value's kind, so the export has no fixed signature)".to_string()),
        other => Err(format!("`{}`, which the generator cannot pass", other.arrow())),
    }
}

/// 戻り値の返し方。
fn ret_abi(ty: &CsTy, classes: &HashMap<String, String>) -> Result<RetAbi, String> {
    Ok(match ty {
        CsTy::Void => RetAbi::Void,
        CsTy::Bool => RetAbi::Long("$r ? 1L : 0L".to_string()),
        CsTy::Int { bits: 64, signed: false } => RetAbi::Long("ArrowAbi.FromUInt64($r, what)".to_string()),
        CsTy::Int { .. } | CsTy::NInt { signed: true } => RetAbi::Long("(long)$r".to_string()),
        CsTy::NInt { signed: false } => RetAbi::Long("ArrowAbi.FromUInt64((ulong)$r, what)".to_string()),
        CsTy::Float { .. } => RetAbi::Long("BitConverter.DoubleToInt64Bits((double)$r)".to_string()),
        CsTy::Char => RetAbi::Str("$r.ToString()".to_string()),
        CsTy::Str => RetAbi::Str("$r".to_string()),
        CsTy::Named { name, .. } => match classes.get(name) {
            Some(_) => RetAbi::Long("ArrowObjects.Store($r)".to_string()),
            None => return Err(format!("`{name}`, not a public class of this assembly")),
        },
        CsTy::Generic { base, args, .. } if is_task(base) && args.len() == 1 => ret_abi(&args[0], classes)?,
        // `int?` などは null を返す取り決めがブリッジに無い（Arrow は呼ぶ前に TypeError にする）。null が来たら誤り
        CsTy::Generic { base, args, .. } if base == "System.Nullable" && args.len() == 1 => match ret_abi(&args[0], classes)? {
            RetAbi::Long(conv) => RetAbi::Long(format!(
                "$r is null ? throw new ArrowBridgeError(\"TypeError: C# method '\" + what + \"' returned null\") : {}",
                conv.replace("$r", "$r.Value")
            )),
            _ => return Err(format!("`{}`, which the generator cannot pass", ty.arrow())),
        },
        CsTy::Object => return Err("`object`, which the bridge cannot read".to_string()),
        other => return Err(format!("`{}`, which the generator cannot pass", other.arrow())),
    })
}

/// `long` から C# の整数型へ（幅を確かめる `ArrowAbi` の関数の名前）。
fn int_conv(bits: u8, signed: bool) -> &'static str {
    match (bits, signed) {
        (8, true) => "ToSByte",
        (8, false) => "ToByte",
        (16, true) => "ToInt16",
        (16, false) => "ToUInt16",
        (32, true) => "ToInt32",
        (32, false) => "ToUInt32",
        _ => "ToUInt64",
    }
}

/// 値型の C# の名前（`Nullable<T>` の中身）。
fn cs_value_type(ty: &CsTy) -> Result<&'static str, String> {
    Ok(match ty {
        CsTy::Bool => "bool?",
        CsTy::Int { bits: 8, signed: true } => "sbyte?",
        CsTy::Int { bits: 8, signed: false } => "byte?",
        CsTy::Int { bits: 16, signed: true } => "short?",
        CsTy::Int { bits: 16, signed: false } => "ushort?",
        CsTy::Int { bits: 32, signed: true } => "int?",
        CsTy::Int { bits: 32, signed: false } => "uint?",
        CsTy::Int { bits: 64, signed: true } => "long?",
        CsTy::Int { .. } => "ulong?",
        CsTy::NInt { signed: true } => "nint?",
        CsTy::NInt { signed: false } => "nuint?",
        CsTy::Float { bits: 32 } => "float?",
        CsTy::Float { .. } => "double?",
        CsTy::Char => "char?",
        other => return Err(format!("`{}?`, which the generator cannot pass", other.arrow())),
    })
}

/// 生成するファイルの先頭（オブジェクト表・ABI の補助・共通の export）。
fn header(source: &str) -> String {
    format!(
        r#"// <auto-generated>
//   Generated by `arrow --gen-cs-exports {source}` (ffi_value_exchange_plan.md 5-1). Do not edit: rebuild the managed
//   DLL and run the command again. Arrow calls these exports for `import[cs-dll]` (src/interpreter/cs_dll_runtime.rs).
// </auto-generated>
#nullable enable
#pragma warning disable CS0219, CS8600, CS8601, CS8602, CS8603, CS8604, CS8625
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

namespace ArrowGenerated;

/// An error found by the bridge itself; the message starts with the Arrow exception type ("TypeError: ...").
internal sealed class ArrowBridgeError : Exception
{{
    public ArrowBridgeError(string message) : base(message) {{ }}
}}

/// C# objects handed to Arrow, by handle (0 is null).
internal static class ArrowObjects
{{
    private static readonly Dictionary<long, object> _table = new();
    private static long _next = 1;

    internal static long Store(object? obj)
    {{
        if (obj is null) return 0;
        long id = Interlocked.Increment(ref _next);
        lock (_table) {{ _table[id] = obj; }}
        return id;
    }}

    /// index 0 is the object a method is called on.
    internal static T Get<T>(long id, int index, string what)
    {{
        string arg = index == 0 ? "the object" : $"argument {{index}}";
        if (id == 0) throw new ArrowBridgeError($"TypeError: {{arg}} of C# method '{{what}}' is None");
        lock (_table)
        {{
            if (_table.TryGetValue(id, out var obj) && obj is T t) return t;
        }}
        throw new ArrowBridgeError($"TypeError: {{arg}} of C# method '{{what}}' is not a live {{typeof(T).Name}} (handle {{id}})");
    }}

    internal static void Release(long id)
    {{
        lock (_table) {{ _table.Remove(id); }}
    }}
}}

internal static unsafe class ArrowAbi
{{
    /// Throws instead of replacing what cannot be encoded (a lone surrogate).
    private static readonly UTF8Encoding Utf8 = new(false, true);

    [ThreadStatic] private static string? _error;

    internal static void SetError(Exception e) =>
        _error = e is ArrowBridgeError ? e.Message : $"{{e.GetType().FullName}}: {{e.Message}}";

    internal static string? TakeError()
    {{
        string? e = _error;
        _error = null;
        return e;
    }}

    internal static string? Str(byte* p, int n) => n < 0 ? null : Utf8.GetString(p, n);

    internal static char Char(byte* p, int n, int index, string what)
    {{
        string? s = Str(p, n);
        if (s is null || s.Length != 1)
            throw new ArrowBridgeError($"ValueError: argument {{index}} of C# method '{{what}}' must be a str of one UTF-16 unit (C# `char`)");
        return s[0];
    }}

    internal static void WriteStr(string? s, byte** outPtr, int* outLen)
    {{
        if (s is null) {{ *outPtr = null; *outLen = -1; return; }}
        byte[] b = Utf8.GetBytes(s);
        byte* p = (byte*)NativeMemory.Alloc((nuint)b.Length + 1);
        b.AsSpan().CopyTo(new Span<byte>(p, b.Length));
        p[b.Length] = 0;
        *outPtr = p;
        *outLen = b.Length;
    }}

    private static ArrowBridgeError Overflow(long v, string type, int index, string what) =>
        new($"OverflowError: argument {{index}} of C# method '{{what}}' is {{v}}, which does not fit in C# `{{type}}`");

    internal static sbyte ToSByte(long v, int i, string w) => v >= sbyte.MinValue && v <= sbyte.MaxValue ? (sbyte)v : throw Overflow(v, "sbyte", i, w);
    internal static byte ToByte(long v, int i, string w) => v >= 0 && v <= byte.MaxValue ? (byte)v : throw Overflow(v, "byte", i, w);
    internal static short ToInt16(long v, int i, string w) => v >= short.MinValue && v <= short.MaxValue ? (short)v : throw Overflow(v, "short", i, w);
    internal static ushort ToUInt16(long v, int i, string w) => v >= 0 && v <= ushort.MaxValue ? (ushort)v : throw Overflow(v, "ushort", i, w);
    internal static int ToInt32(long v, int i, string w) => v >= int.MinValue && v <= int.MaxValue ? (int)v : throw Overflow(v, "int", i, w);
    internal static uint ToUInt32(long v, int i, string w) => v >= 0 && v <= uint.MaxValue ? (uint)v : throw Overflow(v, "uint", i, w);
    internal static ulong ToUInt64(long v, int i, string w) => v >= 0 ? (ulong)v : throw Overflow(v, "ulong", i, w);

    internal static long FromUInt64(ulong v, string what) =>
        v <= long.MaxValue ? (long)v : throw new ArrowBridgeError($"OverflowError: C# method '{{what}}' returned {{v}}, which does not fit in int");
}}

internal static unsafe class ArrowExports
{{
    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_release")]
    public static void Release(long handle) => ArrowObjects.Release(handle);

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_free_str")]
    public static void FreeStr(byte* p) => NativeMemory.Free(p);

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_take_error")]
    public static void TakeError(byte** outPtr, int* outLen) => ArrowAbi.WriteStr(ArrowAbi.TakeError(), outPtr, outLen);
"#
    )
}
