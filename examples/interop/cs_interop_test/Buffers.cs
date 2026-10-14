using System.Runtime.InteropServices;

namespace ArrowBridge;

/// Bytes, pointers, numeric arrays and lists through the generated cs-dll exports (ffi_value_exchange_plan.md 5-5).
public static class Buffers
{
    // byte[] <-> bytes
    public static byte[] Raw(string s) => System.Text.Encoding.UTF8.GetBytes(s);
    public static string FromRaw(byte[] b) => System.Text.Encoding.UTF8.GetString(b);
    public static byte[]? NoBytes() => null;

    // numeric arrays <-> Ptr[T] (a copy in unmanaged memory; D-16)
    public static int[] Squares(int n) { var r = new int[n]; for (int i = 0; i < n; i++) r[i] = i * i; return r; }
    public static long Sum(int[] xs) { long t = 0; foreach (var x in xs) t += x; return t; }
    public static double[] Halves(double[] xs) { var r = new double[xs.Length]; for (int i = 0; i < xs.Length; i++) r[i] = xs[i] / 2; return r; }
    public static long[]? NoArray() => null;
    private static readonly int[] _kept = { 1, 2, 3 };
    public static int[] Kept() => _kept;              // C# keeps this array: Arrow gets a copy (D-11)
    public static void ChangeKept() => _kept[0] = 100;

    // lists (expanded at the boundary; D-16)
    public static List<string> Words(string s) => new(s.Split(' '));
    public static string Join(List<string> xs) => string.Join("|", xs);
    public static bool[] Flags() => new[] { true, false };
    public static long CountTrue(bool[] xs) => xs.Count(x => x);
    public static List<long?> Holes() => new() { 1, null, 3 };
    public static List<TextProcessor> Processors() => new() { new TextProcessor("a"), new TextProcessor("bb") };
    public static long TotalLength(List<TextProcessor> ps) => ps.Sum(p => (long)p.Length);

    // IntPtr <-> a borrowed Ptr[void]
    private static readonly IntPtr _buffer = Marshal.AllocHGlobal(16);
    public static IntPtr Handle() => _buffer;
    public static long SameHandle(IntPtr p) => p == _buffer ? 1 : 0;
}
