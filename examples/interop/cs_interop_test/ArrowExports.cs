using System.Runtime.InteropServices;

namespace ArrowBridge;

// ---------------------------------------------------------------------------
// Object table: manages C# object lifetimes exposed to Arrow as i64 handles.
// ---------------------------------------------------------------------------
internal static class ObjTable
{
    private static readonly Dictionary<long, object> _table = new();
    private static long _next = 1;

    internal static long Store(object obj)
    {
        long id = Interlocked.Increment(ref _next);
        lock (_table) { _table[id] = obj; }
        return id;
    }

    internal static T Get<T>(long id) where T : class
    {
        lock (_table)
        {
            if (_table.TryGetValue(id, out var obj) && obj is T t) return t;
            throw new InvalidOperationException($"Invalid handle {id} for type {typeof(T).Name}");
        }
    }

    internal static void Release(long id)
    {
        lock (_table) { _table.Remove(id); }
    }
}

// ---------------------------------------------------------------------------
// Bridge exports — Arrow ABI:
//   integers/booleans  → i64 (1 = true, 0 = false)
//   floats             → f64 passed as i64 bit-pattern via BitConverter
//   strings            → UTF-8 byte* with length (Arrow allocates via arrow_alloc)
//   objects            → i64 handle into ObjTable
// ---------------------------------------------------------------------------
public static unsafe class ArrowExports
{
    // ── Lifecycle ────────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_release")]
    public static void Release(long handle) => ObjTable.Release(handle);

    // ── Calculator — static methods ──────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "Calculator_new_0")]
    public static long Calculator_new_0()
    {
        return ObjTable.Store(new Calculator());
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_new_1")]
    public static long Calculator_new_1(double initial)
    {
        return ObjTable.Store(new Calculator(initial));
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Add")]
    public static long Calculator_Add(long a, long b)
        => Calculator.Add((int)a, (int)b);

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Subtract")]
    public static long Calculator_Subtract(long a, long b)
        => Calculator.Subtract((int)a, (int)b);

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Multiply")]
    public static long Calculator_Multiply(long a, long b)
        => Calculator.Multiply((int)a, (int)b);

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Divide")]
    public static long Calculator_Divide(long a_bits, long b_bits)
    {
        double a = BitConverter.Int64BitsToDouble(a_bits);
        double b = BitConverter.Int64BitsToDouble(b_bits);
        double r = Calculator.Divide(a, b);
        return BitConverter.DoubleToInt64Bits(r);
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Power")]
    public static long Calculator_Power(long base_bits, long exp_bits)
    {
        double b = BitConverter.Int64BitsToDouble(base_bits);
        double e = BitConverter.Int64BitsToDouble(exp_bits);
        return BitConverter.DoubleToInt64Bits(Calculator.Power(b, e));
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_Sqrt")]
    public static long Calculator_Sqrt(long x_bits)
    {
        double x = BitConverter.Int64BitsToDouble(x_bits);
        return BitConverter.DoubleToInt64Bits(Calculator.Sqrt(x));
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_getPi")]
    public static long Calculator_getPi()
        => BitConverter.DoubleToInt64Bits(Calculator.Pi);

    [UnmanagedCallersOnly(EntryPoint = "Calculator_getE")]
    public static long Calculator_getE()
        => BitConverter.DoubleToInt64Bits(Calculator.E);

    // ── Calculator — instance methods ────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "Calculator_inst_Accumulate")]
    public static void Calculator_inst_Accumulate(long handle, long val_bits)
    {
        double v = BitConverter.Int64BitsToDouble(val_bits);
        ObjTable.Get<Calculator>(handle).Accumulate(v);
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_inst_GetAccumulated")]
    public static long Calculator_inst_GetAccumulated(long handle)
    {
        double r = ObjTable.Get<Calculator>(handle).GetAccumulated();
        return BitConverter.DoubleToInt64Bits(r);
    }

    [UnmanagedCallersOnly(EntryPoint = "Calculator_inst_Reset")]
    public static void Calculator_inst_Reset(long handle)
        => ObjTable.Get<Calculator>(handle).Reset();

    // ── TextProcessor — constructors ─────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_new")]
    public static long TextProcessor_new(byte* text, int textLen)
    {
        string s = Marshal.PtrToStringUTF8((IntPtr)text, textLen);
        return ObjTable.Store(new TextProcessor(s));
    }

    // ── TextProcessor — instance methods ─────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_getText")]
    public static void TextProcessor_inst_getText(long handle, byte** out_ptr, int* out_len)
    {
        string s = ObjTable.Get<TextProcessor>(handle).Text;
        WriteString(s, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_getLength")]
    public static long TextProcessor_inst_getLength(long handle)
        => ObjTable.Get<TextProcessor>(handle).Length;

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_ToUpper")]
    public static void TextProcessor_inst_ToUpper(long handle, byte** out_ptr, int* out_len)
    {
        string r = ObjTable.Get<TextProcessor>(handle).ToUpper();
        WriteString(r, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_ToLower")]
    public static void TextProcessor_inst_ToLower(long handle, byte** out_ptr, int* out_len)
    {
        string r = ObjTable.Get<TextProcessor>(handle).ToLower();
        WriteString(r, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_Trim")]
    public static void TextProcessor_inst_Trim(long handle, byte** out_ptr, int* out_len)
    {
        string r = ObjTable.Get<TextProcessor>(handle).Trim();
        WriteString(r, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_Contains")]
    public static long TextProcessor_inst_Contains(long handle, byte* sub, int subLen)
    {
        string s = Marshal.PtrToStringUTF8((IntPtr)sub, subLen);
        return ObjTable.Get<TextProcessor>(handle).Contains(s) ? 1L : 0L;
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_Replace")]
    public static void TextProcessor_inst_Replace(
        long handle, byte* old_ptr, int old_len, byte* new_ptr, int new_len,
        byte** out_ptr, int* out_len)
    {
        string old = Marshal.PtrToStringUTF8((IntPtr)old_ptr, old_len);
        string nw = Marshal.PtrToStringUTF8((IntPtr)new_ptr, new_len);
        string r = ObjTable.Get<TextProcessor>(handle).Replace(old, nw);
        WriteString(r, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_inst_Append")]
    public static void TextProcessor_inst_Append(long handle, byte* s_ptr, int s_len)
    {
        string s = Marshal.PtrToStringUTF8((IntPtr)s_ptr, s_len);
        ObjTable.Get<TextProcessor>(handle).Append(s);
    }

    // ── TextProcessor — static methods ───────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_Repeat")]
    public static void TextProcessor_Repeat(
        byte* s_ptr, int s_len, long times,
        byte** out_ptr, int* out_len)
    {
        string s = Marshal.PtrToStringUTF8((IntPtr)s_ptr, s_len);
        string r = TextProcessor.Repeat(s, (int)times);
        WriteString(r, out_ptr, out_len);
    }

    [UnmanagedCallersOnly(EntryPoint = "TextProcessor_IsNullOrEmpty")]
    public static long TextProcessor_IsNullOrEmpty(byte* s_ptr, int s_len)
    {
        string? s = s_len < 0 ? null : Marshal.PtrToStringUTF8((IntPtr)s_ptr, s_len);
        return TextProcessor.IsNullOrEmpty(s) ? 1L : 0L;
    }

    // ── String helpers ───────────────────────────────────────────────────────
    // Arrow calls arrow_bridge_free_str to release strings returned here.

    private static readonly List<IntPtr> _allocs = new();

    private static void WriteString(string s, byte** out_ptr, int* out_len)
    {
        byte[] bytes = System.Text.Encoding.UTF8.GetBytes(s);
        IntPtr ptr = Marshal.AllocHGlobal(bytes.Length + 1);
        Marshal.Copy(bytes, 0, ptr, bytes.Length);
        Marshal.WriteByte(ptr + bytes.Length, 0);
        *out_ptr = (byte*)ptr;
        *out_len = bytes.Length;
        lock (_allocs) { _allocs.Add(ptr); }
    }

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_free_str")]
    public static void FreeStr(byte* ptr)
    {
        var p = (IntPtr)ptr;
        lock (_allocs) { _allocs.Remove(p); }
        Marshal.FreeHGlobal(p);
    }
}
