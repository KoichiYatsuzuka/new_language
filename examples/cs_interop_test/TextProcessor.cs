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

    public static string Repeat(string s, int times) => string.Concat(Enumerable.Repeat(s, times));
    public static string Join(string sep, string[] parts) => string.Join(sep, parts);
    public static bool IsNullOrEmpty(string? s) => string.IsNullOrEmpty(s);
}
