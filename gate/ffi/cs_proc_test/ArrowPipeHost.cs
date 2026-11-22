// ArrowPipeHost.cs
// Reusable IPC host: connects Arrow to .NET via a Windows named pipe.
//
// Protocol: newline-delimited JSON
//   Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"Name","mth":"method","hnd":handle,"args":[...]}
//   Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
//   Arg/result tags: "i"=int64, "f"=float64 (NaN / Infinity / -Infinity as strings), "b"=bool, "s"=string,
//   "h"=handle, "n"=null, and the copied containers (ffi_value_exchange_plan.md 5-2): "a"=list, "u"=tuple,
//   "d"=dict as [[key, value], ...], "e"=set, "y"=bytes (base64). Arguments become the parameter's C# type
//   (T[] / List<T> / IEnumerable<T> ..., ValueTuple / Tuple, Dictionary<K,V> ..., HashSet<T> ..., byte[]).
//   Result only: "o" = an integer that does not fit in int64 (ulong >= 2^63), as a decimal string
//   (Arrow raises OverflowError for it; ffi_value_exchange_plan.md 2-1)

using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.IO.Pipes;
using System.Linq;
using System.Reflection;
using System.Text;
using System.Text.Json;

namespace ArrowHost;

internal sealed class ArrowPipeHost
{
    private readonly Assembly[] _assemblies;
    private readonly Dictionary<long, object> _objTable = new();
    private long _nextHandle = 1;

    public ArrowPipeHost(params Assembly[] assemblies)
    {
        _assemblies = assemblies;
    }

    public void Run(string[] args)
    {
        if (args.Length < 1)
        {
            Console.Error.WriteLine("Usage: <prog> <named-pipe>");
            return;
        }

        // Strip the \\.\pipe\ prefix — NamedPipeServerStream takes just the name.
        string fullPipeName = args[0];
        string pipeName = fullPipeName;
        if (pipeName.StartsWith(@"\\.\pipe\", StringComparison.OrdinalIgnoreCase))
            pipeName = pipeName.Substring(@"\\.\pipe\".Length);

        using var server = new NamedPipeServerStream(
            pipeName,
            PipeDirection.InOut,
            maxNumberOfServerInstances: 1,
            transmissionMode: PipeTransmissionMode.Byte,
            options: PipeOptions.None);

        // Signal Arrow that the pipe is ready for connection.
        Console.WriteLine("READY");
        Console.Out.Flush();

        server.WaitForConnection();

        // Use raw stream I/O to avoid StreamReader buffering consuming pipe data.
        while (true)
        {
            string? line = ReadPipeLine(server);
            if (line == null) break;
            line = line.Trim();
            if (line.Length == 0) continue;

            long id = 0;
            try
            {
                using var doc = JsonDocument.Parse(line);
                var req = doc.RootElement;
                id = req.GetProperty("id").GetInt64();
                string op = req.GetProperty("op").GetString()!;

                switch (op)
                {
                    case "quit":
                        WritePipeLine(server, JsonSerializer.Serialize(new { id, ok = (object?)null }));
                        return;

                    case "static":
                    {
                        string cls = req.GetProperty("cls").GetString()!;
                        string mth = req.GetProperty("mth").GetString()!;
                        var argsElem = req.GetProperty("args");
                        object? retVal = InvokeStatic(cls, mth, argsElem);
                        WritePipeLine(server, JsonSerializer.Serialize(new { id, ok = retVal }));
                        break;
                    }

                    case "new":
                    {
                        string cls = req.GetProperty("cls").GetString()!;
                        var argsElem = req.GetProperty("args");
                        object? obj = InvokeConstructor(cls, argsElem);
                        long handle;
                        if (obj == null)
                        {
                            handle = 0;
                        }
                        else
                        {
                            handle = _nextHandle++;
                            _objTable[handle] = obj;
                        }
                        WritePipeLine(server, JsonSerializer.Serialize(new { id, ok = new { t = "h", v = handle } }));
                        break;
                    }

                    case "inst":
                    {
                        string cls = req.GetProperty("cls").GetString()!;
                        string mth = req.GetProperty("mth").GetString()!;
                        long hnd = req.GetProperty("hnd").GetInt64();
                        var argsElem = req.GetProperty("args");
                        _objTable.TryGetValue(hnd, out object? obj);
                        object? retVal = InvokeInstance(obj, cls, mth, argsElem);
                        WritePipeLine(server, JsonSerializer.Serialize(new { id, ok = retVal }));
                        break;
                    }

                    default:
                        throw new Exception($"Unknown op: {op}");
                }
            }
            catch (Exception ex)
            {
                // The exception thrown by the invoked member (reflection wraps it), with its type like cs-dll's
                // generated exports: "System.InvalidOperationException: boom" (Arrow raises it as ForeignError, plan 7-4).
                Exception thrown = ex.InnerException ?? ex;
                string msg = $"{thrown.GetType().FullName}: {thrown.Message}";
                WritePipeLine(server, JsonSerializer.Serialize(new { id, err = msg }));
            }
        }
    }

    // ── Raw pipe line I/O (no StreamReader buffering) ─────────────────────────

    // Arrow sends UTF-8. Collect the bytes of one line and decode them together (plan 2-2): this used to append
    // each byte as a char ((char)b), so every non-ASCII string sent from Arrow arrived garbled.
    // Invalid UTF-8 throws (it cannot come from Arrow, whose strings are always valid).
    private static readonly UTF8Encoding StrictUtf8 = new(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);

    private static string? ReadPipeLine(Stream s)
    {
        var buf = new MemoryStream();
        while (true)
        {
            int b = s.ReadByte();
            if (b == -1) return buf.Length > 0 ? StrictUtf8.GetString(buf.ToArray()) : null;
            if (b == (byte)'\n') return StrictUtf8.GetString(buf.ToArray());
            if (b != (byte)'\r') buf.WriteByte((byte)b);
        }
    }

    private static void WritePipeLine(Stream s, string json)
    {
        byte[] bytes = Encoding.UTF8.GetBytes(json + "\n");
        s.Write(bytes, 0, bytes.Length);
        s.Flush();
    }

    // ── Type lookup ──────────────────────────────────────────────────────────

    private Type FindType(string name)
    {
        foreach (var asm in _assemblies)
        {
            foreach (var t in asm.GetTypes())
            {
                if (t.Name == name && t.IsPublic)
                    return t;
            }
        }
        throw new Exception($"Type '{name}' not found in registered assemblies");
    }

    // ── Dispatch ─────────────────────────────────────────────────────────────

    private object? InvokeStatic(string cls, string mth, JsonElement argsElem)
    {
        var type = FindType(cls);
        int argc = argsElem.GetArrayLength();
        var candidates = type.GetMethods(BindingFlags.Public | BindingFlags.Static)
            .Where(m => m.Name == mth).ToArray();
        var method = candidates.FirstOrDefault(m => m.GetParameters().Length == argc)
            ?? candidates.FirstOrDefault()
            ?? throw new Exception($"Static method '{mth}' not found on '{cls}'");
        var converted = ConvertArgs(argsElem, method.GetParameters());
        return EncodeResult(method.Invoke(null, converted), method.ReturnType);
    }

    private object? InvokeConstructor(string cls, JsonElement argsElem)
    {
        var type = FindType(cls);
        int argc = argsElem.GetArrayLength();
        var ctor = type.GetConstructors()
            .FirstOrDefault(c => c.GetParameters().Length == argc)
            ?? type.GetConstructors().FirstOrDefault()
            ?? throw new Exception($"Constructor for '{cls}' with {argc} args not found");
        var converted = ConvertArgs(argsElem, ctor.GetParameters());
        return ctor.Invoke(converted);
    }

    private object? InvokeInstance(object? obj, string cls, string mth, JsonElement argsElem)
    {
        if (obj == null)
            throw new Exception($"Object handle not found for class '{cls}'");
        var type = obj.GetType();
        int argc = argsElem.GetArrayLength();
        var candidates = type.GetMethods(BindingFlags.Public | BindingFlags.Instance)
            .Where(m => m.Name == mth).ToArray();
        var method = candidates.FirstOrDefault(m => m.GetParameters().Length == argc)
            ?? candidates.FirstOrDefault()
            ?? throw new Exception($"Instance method '{mth}' not found on '{cls}'");
        var converted = ConvertArgs(argsElem, method.GetParameters());
        return EncodeResult(method.Invoke(obj, converted), method.ReturnType);
    }

    // ── Arg conversion ───────────────────────────────────────────────────────

    private object?[] ConvertArgs(JsonElement argsElem, ParameterInfo[] parms)
    {
        int count = Math.Min(argsElem.GetArrayLength(), parms.Length);
        var result = new object?[parms.Length];
        for (int i = 0; i < count; i++)
            result[i] = ConvertArg(argsElem[i], parms[i].ParameterType);
        return result;
    }

    private object? ConvertArg(JsonElement arg, Type targetType)
    {
        // A missing or unknown tag is an error (it used to become null silently; plan 2-1).
        if (!arg.TryGetProperty("t", out var tagElem))
            throw new Exception($"argument without a type tag: {arg}");
        string t = tagElem.GetString() ?? "";
        // `long?` / `double?` parameters take the underlying type (Convert.ChangeType cannot target Nullable<T>).
        Type target = Nullable.GetUnderlyingType(targetType) ?? targetType;

        return t switch
        {
            "i" => Convert.ChangeType(arg.GetProperty("v").GetInt64(), target),
            "f" => Convert.ChangeType(ReadDouble(arg.GetProperty("v")), target),
            "b" => arg.GetProperty("v").GetBoolean(),
            "s" when target == typeof(char) => SingleChar(arg.GetProperty("v").GetString()!),
            "s" => arg.GetProperty("v").GetString(),
            "h" => _objTable.TryGetValue(arg.GetProperty("v").GetInt64(), out var ho)
                ? ho
                : throw new Exception($"object handle {arg.GetProperty("v").GetInt64()} not found"),
            "n" => null,
            "a" or "u" => ConvertSequence(arg.GetProperty("v"), target),
            "e" => ConvertSet(arg.GetProperty("v"), target),
            "d" => ConvertDict(arg.GetProperty("v"), target),
            "y" when target == typeof(byte[]) || target == typeof(object) => Convert.FromBase64String(arg.GetProperty("v").GetString()!),
            "y" => throw new Exception($"bytes cannot become a C# {target.Name}"),
            _   => throw new Exception($"unknown argument type tag '{t}'"),
        };
    }

    private static double ReadDouble(JsonElement v) => v.ValueKind != JsonValueKind.String ? v.GetDouble() : v.GetString() switch
    {
        "NaN" => double.NaN,
        "Infinity" => double.PositiveInfinity,
        "-Infinity" => double.NegativeInfinity,
        var other => throw new Exception($"not a number: '{other}'"),
    };

    // ── Containers (plan 5-2): copies, built as the parameter's C# type ───────

    /// The T of IEnumerable<T> that `t` is or implements (null when it has none).
    private static Type? ElementOf(Type t, Type generic)
    {
        if (t.IsGenericType && t.GetGenericTypeDefinition() == generic) return t.GetGenericArguments()[0];
        return t.GetInterfaces().FirstOrDefault(i => i.IsGenericType && i.GetGenericTypeDefinition() == generic)
            ?.GetGenericArguments()[0];
    }

    private static bool IsTupleType(Type t) =>
        t.IsGenericType && ((t.FullName ?? "").StartsWith("System.ValueTuple`") || (t.FullName ?? "").StartsWith("System.Tuple`"));

    /// A collection of `concrete` (built with its parameterless constructor and ICollection<T>.Add), or `fallback`
    /// when `target` accepts it (an interface such as IList<T> / IEnumerable<T>).
    private static object NewCollection(Type target, Type fallback, Type elem, string what)
    {
        if (target.IsAssignableFrom(fallback)) return Activator.CreateInstance(fallback)!;
        if (!target.IsAbstract && target.GetConstructor(Type.EmptyTypes) != null
            && typeof(ICollection<>).MakeGenericType(elem).IsAssignableFrom(target))
            return Activator.CreateInstance(target)!;
        throw new Exception($"a {what} cannot become a C# {target.Name}");
    }

    private object? ConvertSequence(JsonElement items, Type target)
    {
        int n = items.GetArrayLength();
        if (target == typeof(object))
            return Enumerable.Range(0, n).Select(i => ConvertArg(items[i], typeof(object))).ToList();
        if (target.IsArray)
        {
            Type et = target.GetElementType()!;
            var arr = Array.CreateInstance(et, n);
            for (int i = 0; i < n; i++) arr.SetValue(ConvertArg(items[i], et), i);
            return arr;
        }
        if (IsTupleType(target))
        {
            var types = target.GetGenericArguments();
            if (types.Length != n || n > 7)
                throw new Exception($"a tuple of {n} cannot become a C# {target.Name}");
            var vals = Enumerable.Range(0, n).Select(i => ConvertArg(items[i], types[i])).ToArray();
            return Activator.CreateInstance(target, vals);
        }
        Type elem = ElementOf(target, typeof(IEnumerable<>)) ?? throw new Exception($"a list cannot become a C# {target.Name}");
        object coll = NewCollection(target, typeof(List<>).MakeGenericType(elem), elem, "list");
        var add = typeof(ICollection<>).MakeGenericType(elem).GetMethod("Add")!;
        for (int i = 0; i < n; i++) add.Invoke(coll, new[] { ConvertArg(items[i], elem) });
        return coll;
    }

    private object? ConvertSet(JsonElement items, Type target)
    {
        int n = items.GetArrayLength();
        Type elem = target == typeof(object) ? typeof(object)
            : ElementOf(target, typeof(IEnumerable<>)) ?? throw new Exception($"a set cannot become a C# {target.Name}");
        object set = target == typeof(object) ? new HashSet<object?>()
            : NewCollection(target, typeof(HashSet<>).MakeGenericType(elem), elem, "set");
        var add = typeof(ICollection<>).MakeGenericType(elem).GetMethod("Add")!;
        for (int i = 0; i < n; i++) add.Invoke(set, new[] { ConvertArg(items[i], elem) });
        return set;
    }

    private object? ConvertDict(JsonElement pairs, Type target)
    {
        Type kt = typeof(object), vt = typeof(object);
        var dictIface = target == typeof(object) ? null
            : target.IsGenericType && target.GetGenericTypeDefinition() == typeof(IDictionary<,>) ? target
            : target.IsGenericType && target.GetGenericTypeDefinition() == typeof(IReadOnlyDictionary<,>) ? target
            : target.GetInterfaces().FirstOrDefault(i => i.IsGenericType && i.GetGenericTypeDefinition() == typeof(IDictionary<,>));
        if (target != typeof(object))
        {
            if (dictIface == null) throw new Exception($"a dict cannot become a C# {target.Name}");
            var ga = dictIface.GetGenericArguments();
            (kt, vt) = (ga[0], ga[1]);
        }
        var concrete = typeof(Dictionary<,>).MakeGenericType(kt, vt);
        object dict = target == typeof(object) || target.IsAssignableFrom(concrete)
            ? Activator.CreateInstance(concrete)!
            : !target.IsAbstract && target.GetConstructor(Type.EmptyTypes) != null ? Activator.CreateInstance(target)!
            : throw new Exception($"a dict cannot become a C# {target.Name}");
        // Add (not the indexer): two keys that become equal in C# are an error, not one value silently lost
        var add = typeof(IDictionary<,>).MakeGenericType(kt, vt).GetMethod("Add")!;
        foreach (var pair in pairs.EnumerateArray())
            add.Invoke(dict, new[] { ConvertArg(pair[0], kt), ConvertArg(pair[1], vt) });
        return dict;
    }

    private static char SingleChar(string s)
    {
        if (s.Length != 1)
            throw new Exception($"a C# char takes a string of length 1, got {s.Length} characters");
        return s[0];
    }

    // ── Result encoding ──────────────────────────────────────────────────────

    private object? EncodeResult(object? result, Type returnType)
    {
        if (returnType == typeof(void) || result == null) return null;
        // Task<T> / ValueTask<T>: the stub declares T, so wait for it (plan 5-2: the host used to return the Task as a
        // handle, which Arrow rejected)
        if (returnType.IsGenericType && returnType.GetGenericTypeDefinition() == typeof(ValueTask<>))
            return EncodeResult(returnType.GetMethod("AsTask")!.Invoke(result, null), typeof(Task<>).MakeGenericType(returnType.GetGenericArguments()));
        if (result is ValueTask vt) { vt.GetAwaiter().GetResult(); return null; }
        if (result is Task task)
        {
            task.GetAwaiter().GetResult();
            if (!returnType.IsGenericType || returnType.GetGenericTypeDefinition() != typeof(Task<>)) return null;
            var inner = returnType.GetGenericArguments()[0];
            return EncodeResult(returnType.GetProperty("Result")!.GetValue(task), inner);
        }
        // Every primitive goes back by value. Only class instances become handles (plan 2-1: char / byte / short /
        // ushort / sbyte / ulong used to fall into StoreHandle and reached Arrow as a handle number).
        return result switch
        {
            string s => new { t = "s", v = s },
            char   c => new { t = "s", v = c.ToString() },
            int    i => new { t = "i", v = (long)i },
            long   l => new { t = "i", v = l },
            uint   u => new { t = "i", v = (long)u },
            short  h => new { t = "i", v = (long)h },
            ushort h => new { t = "i", v = (long)h },
            byte   y => new { t = "i", v = (long)y },
            sbyte  y => new { t = "i", v = (long)y },
            ulong  u => u <= long.MaxValue
                ? new { t = "i", v = (object)(long)u }
                : new { t = "o", v = (object)u.ToString() },
            double d => new { t = "f", v = d },
            float  f => new { t = "f", v = (double)f },
            bool   b => new { t = "b", v = b },
            _        => EncodeObject(result),
        };
    }

    /// Copied containers (plan 5-2); anything else is an object of this assembly, handed out as a handle.
    private object? EncodeObject(object result)
    {
        object? Item(object? x) => EncodeResult(x, x?.GetType() ?? typeof(object));
        var type = result.GetType();
        if (result is byte[] bytes) return new { t = "y", v = Convert.ToBase64String(bytes) };
        if (IsTupleType(type) && result is System.Runtime.CompilerServices.ITuple tup)
            return new { t = "u", v = Enumerable.Range(0, tup.Length).Select(i => Item(tup[i])).ToList() };
        if (type.IsGenericType && type.GetGenericTypeDefinition() == typeof(KeyValuePair<,>))
            return new { t = "u", v = new[] { Item(type.GetProperty("Key")!.GetValue(result)), Item(type.GetProperty("Value")!.GetValue(result)) } };
        if (result is IDictionary dict)
        {
            // IDictionary.GetEnumerator gives DictionaryEntry (the non-generic IEnumerable of Dictionary<K,V> gives KeyValuePair)
            var pairs = new List<object?[]>();
            var en = dict.GetEnumerator();
            while (en.MoveNext()) pairs.Add(new[] { Item(en.Key), Item(en.Value) });
            return new { t = "d", v = pairs };
        }
        if (ElementOf(type, typeof(ISet<>)) != null || ElementOf(type, typeof(IReadOnlySet<>)) != null)
            return new { t = "e", v = ((IEnumerable)result).Cast<object?>().Select(Item).ToList() };
        if (result is IEnumerable seq)
            return new { t = "a", v = seq.Cast<object?>().Select(Item).ToList() };
        return StoreHandle(result);
    }

    private object StoreHandle(object obj)
    {
        long h = _nextHandle++;
        _objTable[h] = obj;
        return new { t = "h", v = h };
    }
}
