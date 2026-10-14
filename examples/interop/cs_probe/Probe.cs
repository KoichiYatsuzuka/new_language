// Probe.cs - C# API for checking how values cross the cs-proc / cs-dll boundary
// (implementation_plans/ffi_value_exchange_plan.md, task 1-1).
//
// Some members are NOT handled correctly by the current bridge (plan section 0.2):
// collections and object returns come back as integer handles, list arguments arrive as null,
// and non-ASCII strings sent from Arrow are garbled. The examples are added by the fixing tasks.
using System.Collections.Generic;

public class Person
{
    public string Name;
    public long Age;
    public Person(string name, long age) { Name = name; Age = age; }
    public string hello() => "I am " + Name;
    public string get_name() => Name;
    public Person older(long by) => new Person(Name + "+", Age + by);
    public Person? best_friend() => null;                 // nullable class return (task 2-1)
    public string[] nicknames() => new[] { Name };        // collection return: a static error (task 2-1)
}

public class Probe
{
    // strings
    public static string greet() => "hello from C#";
    public static string echo(string s) => s;
    public static string utf8() => "日本語";
    public static string? nothing() => null;
    public static long byte_len(string s) => System.Text.Encoding.UTF8.GetByteCount(s);

    // collections
    public static string[] words(string s) => s.Split(' ');
    public static List<string> words_list(string s) => new List<string>(s.Split(' '));
    public static Dictionary<string, long> counts(string s)
    {
        var d = new Dictionary<string, long>();
        foreach (var c in s) { var k = c.ToString(); d[k] = d.TryGetValue(k, out var v) ? v + 1 : 1; }
        return d;
    }
    public static long[] ints() => new long[] { 1, 2, 3 };
    public static long sum(long[] xs) { long t = 0; foreach (var x in xs) t += x; return t; }
    public static string join(List<string> xs) => string.Join("|", xs);

    // objects
    public static Person make_person(string name) => new Person(name, 20);
    public static string describe(Person p) => p == null ? "null" : p.Name;

    // nullable annotations (task 2-1): `string?` / `Person?` / `long?` become Option[...] in the stub
    public static string? maybe(bool b) => b ? "yes" : null;
    public static long? maybe_num(bool b) => b ? 7 : null;
    public static Person? find(string name) => name == "nobody" ? null : new Person(name, 30);
    public static string describe_opt(Person? p) => p == null ? "none" : p.Name;
    public static long add_opt(long a, long? b) => a + (b ?? 100);
    public static string broken_contract() => null!;     // declared non-null, returns null

    // small value types (task 2-1): the host used to send these back as object handles
    public static char initial(string s) => s[0];
    public static string twice(char c) => new string(c, 2);
    public static byte low_byte(long v) => (byte)v;
    public static short neg_short() => -12;
    public static ushort big_ushort() => 65000;
    public static ulong huge_ulong() => ulong.MaxValue;
    public static ulong small_ulong() => 9;

    // `object` returns: primitives come back by value, objects of unknown class are an error
    public static object any_str() => "object string";
    public static object any_obj() => new Person("anon", 1);

    // collections / bytes / Task<T> copied through cs-proc (task 5-2)
    public static byte[] raw(string s) => System.Text.Encoding.UTF8.GetBytes(s);
    public static string from_raw(byte[] b) => System.Text.Encoding.UTF8.GetString(b);
    public static HashSet<long> uniq(long[] xs) => new HashSet<long>(xs);
    public static long set_size(HashSet<string> s) => s.Count;
    public static string pair_text((long, string) p) => p.Item1 + ":" + p.Item2;
    public static List<Person> people() => new List<Person> { new Person("a", 1), new Person("b", 2) };
    public static long total_age(List<Person> ps) { long t = 0; foreach (var p in ps) t += p.Age; return t; }
    public static Dictionary<long, List<string>> groups() =>
        new Dictionary<long, List<string>> { [1] = new List<string> { "x" }, [2] = new List<string> { "y", "z" } };
    public static long dict_total(Dictionary<string, long> d) { long t = 0; foreach (var v in d.Values) t += v; return t; }
    public static double[] halves(double[] xs) { var r = new double[xs.Length]; for (int i = 0; i < xs.Length; i++) r[i] = xs[i] / 2; return r; }
    public static List<long?> holes() => new List<long?> { 1, null, 3 };
    public static string[] with_null() => new string[] { "a", null! };   // declared string[]: a null element breaks it
    public static System.Threading.Tasks.Task<long> later(long x) => System.Threading.Tasks.Task.Run(() => x * 2);

    // signatures Arrow cannot use yet (task 2-1: static errors)
    public static System.Text.StringBuilder builder() => new System.Text.StringBuilder("sb");
    public static T first<T>(T[] xs) => xs[0];
    public static long apply(System.Func<long, long> f, long x) => f(x);
    public static (long, string) pair() => (1, "one");
}
