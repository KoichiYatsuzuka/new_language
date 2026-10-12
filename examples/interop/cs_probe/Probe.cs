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
}
