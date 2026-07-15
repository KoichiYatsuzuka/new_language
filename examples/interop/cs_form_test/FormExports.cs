using System;
using System.Runtime.InteropServices;
using System.Threading;
using System.Windows.Forms;

namespace FormBridge;

// ── Stub class (for Arrow metadata generation) ────────────────────────────────
// cs_assembly.rs reads THIS class's C# return types to generate Arrow stubs.
// The actual runtime dispatch goes to the FormBridgeExports class below via
// the matching [UnmanagedCallersOnly(EntryPoint = "FormApp_*")] exports.

/// <summary>
/// Arrow-visible stub class. C# return types determine Arrow's return_type
/// so the bridge dispatcher knows whether to call call_returning_str() etc.
/// </summary>
public static class FormApp
{
    /// <summary>Show a MessageBox. buttons: 0=OK 1=OKCancel 2=YesNo 3=YesNoCancel.
    /// Returns: 0=OK 1=Yes 2=No 3=Cancel.</summary>
    public static int  message_box(string title, string message, int buttons) => 0;

    /// <summary>One-line input dialog. Returns 0 if cancelled, else string handle.</summary>
    public static long input_box(string title, string prompt) => 0;

    /// <summary>Note creation form. Returns 0 if cancelled, else note handle.</summary>
    public static long show_note(string title) => 0;

    /// <summary>Get title from note handle.</summary>
    public static string get_note_title(long handle) => "";

    /// <summary>Get content from note handle.</summary>
    public static string get_note_content(long handle) => "";

    /// <summary>TODO manager form. Returns handle to task list.</summary>
    public static long show_todo(string title) => 0;

    /// <summary>Task count from todo handle.</summary>
    public static long todo_count(long handle) => 0;

    /// <summary>Get task string at index from todo handle.</summary>
    public static string todo_get(long handle, long index) => "";

    /// <summary>Retrieve a string from a string handle (input_box result).</summary>
    public static string get_str(long handle) => "";

    /// <summary>Release an object handle.</summary>
    public static void release(long handle) { }
}

// ── Bridge export class (raw pointer ABI) ────────────────────────────────────
// These [UnmanagedCallersOnly] functions are the REAL implementations.
// Arrow runtime looks up e.g. "FormApp_message_box" in the native DLL and
// calls it with the low-level ABI (string → ptr+len pairs, etc.).

public static unsafe class FormBridgeExports
{
    // ── Lifecycle ───────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_release")]
    public static void release(long handle) => ObjTable.Release(handle);

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_release")]
    public static void arrow_release(long handle) => ObjTable.Release(handle);

    [UnmanagedCallersOnly(EntryPoint = "arrow_bridge_free_str")]
    public static void free_str(byte* ptr)
    {
        if (ptr != null) Marshal.FreeHGlobal((IntPtr)ptr);
    }

    // ── message_box ──────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_message_box")]
    public static long message_box(
        byte* title_ptr, int title_len,
        byte* msg_ptr,   int msg_len,
        long  buttons)
    {
        string title = Utf8(title_ptr, title_len);
        string msg   = Utf8(msg_ptr,   msg_len);

        var btns = buttons switch
        {
            1 => MessageBoxButtons.OKCancel,
            2 => MessageBoxButtons.YesNo,
            3 => MessageBoxButtons.YesNoCancel,
            _ => MessageBoxButtons.OK,
        };

        DialogResult result = DialogResult.None;
        RunSta(() =>
        {
            result = MessageBox.Show(msg, title, btns, MessageBoxIcon.Information);
        });

        return result switch
        {
            DialogResult.OK     => 0,
            DialogResult.Yes    => 1,
            DialogResult.No     => 2,
            DialogResult.Cancel => 3,
            _ => 0,
        };
    }

    // ── input_box ────────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_input_box")]
    public static long input_box(
        byte* title_ptr,  int title_len,
        byte* prompt_ptr, int prompt_len)
    {
        string title  = Utf8(title_ptr,  title_len);
        string prompt = Utf8(prompt_ptr, prompt_len);

        string? input = null;
        RunSta(() =>
        {
            var form = new InputBoxForm(title, prompt);
            form.ShowDialog();
            if (form.Confirmed) input = form.Input;
        });

        return input == null ? 0L : ObjTable.Store(input);
    }

    // ── show_note ────────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_show_note")]
    public static long show_note(byte* title_ptr, int title_len)
    {
        string title = Utf8(title_ptr, title_len);
        NoteForm? form = null;
        RunSta(() =>
        {
            form = new NoteForm(title);
            form.ShowDialog();
        });
        if (form == null || !form.Submitted) return 0L;
        return ObjTable.Store(new NoteResult(form.NoteTitle, form.NoteContent));
    }

    [UnmanagedCallersOnly(EntryPoint = "FormApp_get_note_title")]
    public static void get_note_title(long handle, byte** out_ptr, int* out_len)
        => WriteStr(ObjTable.Get<NoteResult>(handle).Title, out_ptr, out_len);

    [UnmanagedCallersOnly(EntryPoint = "FormApp_get_note_content")]
    public static void get_note_content(long handle, byte** out_ptr, int* out_len)
        => WriteStr(ObjTable.Get<NoteResult>(handle).Content, out_ptr, out_len);

    // ── show_todo ────────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_show_todo")]
    public static long show_todo(byte* title_ptr, int title_len)
    {
        string title = Utf8(title_ptr, title_len);
        TodoForm? form = null;
        RunSta(() =>
        {
            form = new TodoForm(title);
            form.ShowDialog();
        });
        var tasks = form?.Tasks ?? new System.Collections.Generic.List<string>();
        return ObjTable.Store(new TodoResult(tasks));
    }

    [UnmanagedCallersOnly(EntryPoint = "FormApp_todo_count")]
    public static long todo_count(long handle)
        => ObjTable.Get<TodoResult>(handle).Tasks.Count;

    [UnmanagedCallersOnly(EntryPoint = "FormApp_todo_get")]
    public static void todo_get(long handle, long index, byte** out_ptr, int* out_len)
    {
        var tasks = ObjTable.Get<TodoResult>(handle).Tasks;
        int i = (int)index;
        WriteStr(i >= 0 && i < tasks.Count ? tasks[i] : "", out_ptr, out_len);
    }

    // ── get_str ──────────────────────────────────────────────────────────────

    [UnmanagedCallersOnly(EntryPoint = "FormApp_get_str")]
    public static void get_str(long handle, byte** out_ptr, int* out_len)
        => WriteStr(ObjTable.Get<string>(handle), out_ptr, out_len);

    // ── Helpers ──────────────────────────────────────────────────────────────

    private static string Utf8(byte* ptr, int len)
        => ptr == null ? "" : Marshal.PtrToStringUTF8((IntPtr)ptr, len) ?? "";

    private static void WriteStr(string s, byte** out_ptr, int* out_len)
    {
        byte[] bytes = System.Text.Encoding.UTF8.GetBytes(s);
        IntPtr ptr = Marshal.AllocHGlobal(bytes.Length + 1);
        Marshal.Copy(bytes, 0, ptr, bytes.Length);
        Marshal.WriteByte(ptr + bytes.Length, 0);
        *out_ptr = (byte*)ptr;
        *out_len = bytes.Length;
    }

    private static void RunSta(Action action)
    {
        var t = new Thread(() =>
        {
            Application.EnableVisualStyles();
            action();
        });
        t.SetApartmentState(ApartmentState.STA);
        t.Start();
        t.Join();
    }
}

// ── Result record types ───────────────────────────────────────────────────────
internal sealed record NoteResult(string Title, string Content);
internal sealed record TodoResult(System.Collections.Generic.List<string> Tasks);
