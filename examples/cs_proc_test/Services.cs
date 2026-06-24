// Services.cs — Demo service classes exposed to Arrow via import[cs-proc].
// Regular C#: no unsafe code, no NativeAOT required.

using System;
using System.Linq;

/// <summary>A simple accumulator with static helpers.</summary>
public class Calculator
{
    private long _value;

    public Calculator(long initial = 0) => _value = initial;

    // Static helpers
    public static long add(long a, long b)      => a + b;
    public static long subtract(long a, long b) => a - b;
    public static long multiply(long a, long b) => a * b;
    public static long square(long n)           => n * n;
    public static double sqrt(double n)         => Math.Sqrt(n);

    // Instance mutators
    public long increment(long amount) { _value += amount; return _value; }
    public long multiply_by(long factor) { _value *= factor; return _value; }
    public void reset() => _value = 0;

    // Instance accessors
    public long get_value()         => _value;
    public string get_formatted()   => $"Value: {_value}";
    public bool is_positive()       => _value > 0;
    public bool is_zero()           => _value == 0;
}

/// <summary>String utilities exposed to Arrow.</summary>
public class TextProcessor
{
    private string _text;

    public TextProcessor(string text) => _text = text;

    // Static helpers
    public static string join(string sep, string a, string b) => string.Join(sep, a, b);
    public static string format_number(long n)                => n.ToString("N0");
    public static long parse_int(string s)                    => long.Parse(s.Trim());

    // Instance methods
    public string to_upper()  => _text.ToUpper();
    public string to_lower()  => _text.ToLower();
    public string trim()      => _text.Trim();
    public string reverse()   => new string(_text.Reverse().ToArray());
    public long word_count()  => (long)_text.Split(' ', StringSplitOptions.RemoveEmptyEntries).Length;
    public long length()      => (long)_text.Length;
    public bool contains(string sub)                  => _text.Contains(sub);
    public string replace(string old_val, string new_val) => _text.Replace(old_val, new_val);
    public string get_text()  => _text;
    public void set_text(string text) => _text = text;
}
