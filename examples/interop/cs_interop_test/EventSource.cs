namespace ArrowBridge;

// ---------------------------------------------------------------------------
// EventBridge — Arrow の ar_event_fire 関数ポインタを保持し、C# 側から
// Arrow のシグナルを発火する内部ヘルパー。
//
// arrow.exe は DLL ロード直後に arrow_bridge_set_event_fire(ptr) を呼んで
// ar_event_fire のアドレスを注入する (src/interpreter/cs_dll_runtime.rs)。
// 注入前に Fire しても何も起きない (null ガード)。
// ---------------------------------------------------------------------------
internal static unsafe class EventBridge
{
    private static delegate* unmanaged<ulong, byte*, nuint, void> _fire;

    [System.Runtime.InteropServices.UnmanagedCallersOnly(EntryPoint = "arrow_bridge_set_event_fire")]
    public static void SetEventFire(nint fnPtr)
        => _fire = (delegate* unmanaged<ulong, byte*, nuint, void>)fnPtr;

    /// <summary>
    /// signalId (Arrow 側の sig.external_id) に UTF-8 ペイロードを発火する。
    /// 任意のスレッドから呼べる (ar_event_fire はスレッドセーフ)。
    /// </summary>
    internal static void Fire(ulong signalId, string payload)
    {
        if (_fire == null) return;
        var bytes = System.Text.Encoding.UTF8.GetBytes(payload);
        fixed (byte* p = bytes) _fire(signalId, p, (nuint)bytes.Length);
    }
}

// ---------------------------------------------------------------------------
// EventSource — 背景スレッドから Arrow のシグナルを発火するデモクラス。
// ---------------------------------------------------------------------------
public static class EventSource
{
    /// <summary>
    /// 背景スレッドから count 回、intervalMs 間隔で "tick N" を発火する。
    /// Arrow 側は EventLoop.run(timeout) で受け取る。
    /// </summary>
    public static void StartTimer(long signalId, int count, int intervalMs)
    {
        new Thread(() =>
        {
            for (int i = 0; i < count; i++)
            {
                Thread.Sleep(intervalMs);
                EventBridge.Fire((ulong)signalId, $"tick {i + 1}");
            }
        })
        { IsBackground = true }.Start();
    }

    /// <summary>指定ペイロードを 1 回だけ即時発火する (背景スレッド経由)。</summary>
    public static void FireOnce(long signalId, string payload)
    {
        new Thread(() => EventBridge.Fire((ulong)signalId, payload))
        { IsBackground = true }.Start();
    }
}
