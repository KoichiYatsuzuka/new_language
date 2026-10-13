// ArrowPipeHost.cs
// Reusable IPC host: connects Arrow to .NET via a Windows named pipe.
//
// Protocol: newline-delimited JSON
//   Request:  {"id":N,"op":"static"|"new"|"inst"|"quit","cls":"Name","mth":"method","hnd":handle,"args":[...]}
//   Response: {"id":N,"ok":<value>} | {"id":N,"err":"message"}
//   Arg/result tags: "i"=int64, "f"=float64, "b"=bool, "s"=string, "h"=handle, "n"=null
//   Result only: "o" = an integer that does not fit in int64 (ulong >= 2^63), as a decimal string
//   (Arrow raises OverflowError for it; ffi_value_exchange_plan.md 2-1)

using System;
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
                string msg = ex.InnerException?.Message ?? ex.Message;
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
            "f" => Convert.ChangeType(arg.GetProperty("v").GetDouble(), target),
            "b" => arg.GetProperty("v").GetBoolean(),
            "s" when target == typeof(char) => SingleChar(arg.GetProperty("v").GetString()!),
            "s" => arg.GetProperty("v").GetString(),
            "h" => _objTable.TryGetValue(arg.GetProperty("v").GetInt64(), out var ho)
                ? ho
                : throw new Exception($"object handle {arg.GetProperty("v").GetInt64()} not found"),
            "n" => null,
            _   => throw new Exception($"unknown argument type tag '{t}'"),
        };
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
            _        => StoreHandle(result),
        };
    }

    private object StoreHandle(object obj)
    {
        long h = _nextHandle++;
        _objTable[h] = obj;
        return new { t = "h", v = h };
    }
}
