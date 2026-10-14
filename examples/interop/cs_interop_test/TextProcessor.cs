namespace ArrowBridge;

public class TextProcessor
{
    private string _text;

    public TextProcessor(string text)
    {
        _text = text;
    }

    public string Text => _text;
    public int Length => _text.Length;

    public string ToUpper() => _text.ToUpper();
    public string ToLower() => _text.ToLower();
    public string Trim() => _text.Trim();
    public bool Contains(string sub) => _text.Contains(sub);
    public string Replace(string old, string new_) => _text.Replace(old, new_);
    public string[] Split(string sep) => _text.Split(sep);
    public void Append(string s) { _text += s; }
    // object / nullable returns (ffi_value_exchange_plan.md 2-1)
    public TextProcessor Clone() => new TextProcessor(_text);
    public string? FindWord(string word) => _text.Contains(word) ? word : null;

    public static string Repeat(string s, int times) => string.Concat(Enumerable.Repeat(s, times));
    public static string Join(string sep, string[] parts) => string.Join(sep, parts);
    public static bool IsNullOrEmpty(string? s) => string.IsNullOrEmpty(s);

    // `char` is a str of one UTF-16 unit (ffi_value_exchange_plan.md 5-1)
    public char FirstChar() => _text[0];
    public static string RepeatChar(char c, int times) => new string(c, times);
}
