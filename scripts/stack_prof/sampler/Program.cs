// sampler — スタックのサンプリングプロファイラ（管理者権限なし・scripts/prof_stacks.ps1 が作って呼ぶ）
//
//   sampler <out.folded> <timeoutSec> <exe> [args...]
//
// 対象を起動し、約 1 ms ごとに全スレッドを止めてスタックを歩き（dbghelp の StackWalk64）、「根;...;葉 回数」の折りたたみ
// 形式で <out.folded> に書く。記号は exe の隣の PDB から引く。環境変数と作業ディレクトリは呼び出し側のものを引き継ぐ。
// 対象の標準出力・標準エラーは <out.folded>.stdout へ。timeoutSec を超えたらプロセスの木ごと止めて 3 を返す。
// 集計は scripts/stack_prof/analyze.py。
using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Text;

unsafe static class Sampler
{
    [DllImport("kernel32", SetLastError = true)] static extern IntPtr OpenThread(uint access, bool inherit, uint tid);
    [DllImport("kernel32")] static extern uint SuspendThread(IntPtr h);
    [DllImport("kernel32")] static extern uint ResumeThread(IntPtr h);
    [DllImport("kernel32")] static extern bool GetThreadContext(IntPtr h, void* ctx);
    [DllImport("kernel32")] static extern bool CloseHandle(IntPtr h);
    [DllImport("winmm")] static extern uint timeBeginPeriod(uint ms);

    [DllImport("dbghelp")] static extern uint SymSetOptions(uint opts);
    [DllImport("dbghelp", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern bool SymInitializeW(IntPtr hProcess, string searchPath, bool invade);
    [DllImport("dbghelp")] static extern bool SymRefreshModuleList(IntPtr hProcess);
    [DllImport("dbghelp", SetLastError = true)]
    static extern bool SymFromAddr(IntPtr hProcess, ulong addr, out ulong disp, void* sym);
    [DllImport("dbghelp")]
    static extern bool StackWalk64(uint machine, IntPtr hProcess, IntPtr hThread, void* frame, void* ctx,
        IntPtr readMem, IntPtr funcTable, IntPtr modBase, IntPtr translate);

    const int CTX_SIZE = 1232;
    const uint CONTEXT_CONTROL_INTEGER = 0x100003;

    static IntPtr hProc;
    static readonly Dictionary<ulong, string> symCache = new();
    static Process proc;
    static List<(ulong lo, ulong hi, string name)> modules = new();

    static void RefreshModules()
    {
        try
        {
            proc.Refresh();
            var l = new List<(ulong, ulong, string)>();
            foreach (ProcessModule m in proc.Modules)
                l.Add(((ulong)m.BaseAddress, (ulong)m.BaseAddress + (ulong)m.ModuleMemorySize, m.ModuleName));
            modules = l;
        }
        catch { }
    }

    static string ModuleOf(ulong addr)
    {
        foreach (var (lo, hi, name) in modules) if (addr >= lo && addr < hi) return name;
        return null;
    }

    static string Symbolize(ulong addr)
    {
        if (symCache.TryGetValue(addr, out var s)) return s;
        var mod = ModuleOf(addr);
        if (mod == null) { RefreshModules(); SymRefreshModuleList(hProc); mod = ModuleOf(addr) ?? "?"; }
        byte* buf = stackalloc byte[88 + 1024];
        new Span<byte>(buf, 88 + 1024).Clear();
        *(uint*)buf = 88;            // SizeOfStruct
        *(uint*)(buf + 80) = 1000;   // MaxNameLen
        string name;
        if (SymFromAddr(hProc, addr, out _, buf))
        {
            int len = (int)*(uint*)(buf + 76);
            name = Encoding.UTF8.GetString(buf + 84, len);
        }
        else name = "?";
        s = mod + "!" + name;
        symCache[addr] = s;
        return s;
    }

    static int Main(string[] args)
    {
        if (args.Length < 3) { Console.Error.WriteLine("usage: sampler <out.folded> <timeoutSec> <exe> [args...]"); return 2; }
        string outPath = args[0];
        int timeoutSec = int.Parse(args[1]);
        var psi = new ProcessStartInfo(args[2]) { UseShellExecute = false, RedirectStandardOutput = true, RedirectStandardError = true };
        for (int i = 3; i < args.Length; i++) psi.ArgumentList.Add(args[i]);
        proc = Process.Start(psi);
        var so = proc.StandardOutput.ReadToEndAsync();
        var se = proc.StandardError.ReadToEndAsync();
        hProc = proc.Handle;
        timeBeginPeriod(1);
        SymSetOptions(0x00000002 | 0x00000004 | 0x00000010); // UNDNAME | DEFERRED_LOADS | LOAD_LINES
        Thread.Sleep(30);
        if (!SymInitializeW(hProc, Path.GetDirectoryName(Path.GetFullPath(args[2])), true))
            Console.Error.WriteLine("SymInitialize failed: " + Marshal.GetLastWin32Error());
        RefreshModules();

        IntPtr dbghelp = NativeLibrary.Load("dbghelp");
        IntPtr fta = NativeLibrary.GetExport(dbghelp, "SymFunctionTableAccess64");
        IntPtr gmb = NativeLibrary.GetExport(dbghelp, "SymGetModuleBase64");

        void* ctx = NativeMemory.AlignedAlloc(CTX_SIZE, 16);
        byte* frame = (byte*)NativeMemory.Alloc(512);
        var stacks = new Dictionary<string, int>();
        var threads = new Dictionary<uint, IntPtr>();
        var sw = Stopwatch.StartNew();
        long lastThreadScan = -1000;
        long samples = 0;
        var addrs = new List<ulong>(128);
        var lastRip = new Dictionary<uint, (ulong rip, string key)>();
        var sb = new StringBuilder();
        bool timedOut = false;

        while (!proc.HasExited)
        {
            if (sw.ElapsedMilliseconds > timeoutSec * 1000L) { timedOut = true; break; }
            if (sw.ElapsedMilliseconds - lastThreadScan > 250)
            {
                lastThreadScan = sw.ElapsedMilliseconds;
                try
                {
                    proc.Refresh();
                    foreach (ProcessThread t in proc.Threads)
                    {
                        uint tid = (uint)t.Id;
                        if (!threads.ContainsKey(tid))
                        {
                            IntPtr h = OpenThread(0x0002 | 0x0008 | 0x0040, false, tid); // SUSPEND_RESUME | GET_CONTEXT | QUERY_INFORMATION
                            if (h != IntPtr.Zero) threads[tid] = h;
                        }
                    }
                }
                catch { }
            }
            foreach (var kv in threads)
            {
                IntPtr th = kv.Value;
                if (SuspendThread(th) == 0xFFFFFFFF) continue;
                addrs.Clear();
                new Span<byte>(ctx, CTX_SIZE).Clear();
                *(uint*)((byte*)ctx + 0x30) = CONTEXT_CONTROL_INTEGER;
                string reuse = null;
                if (GetThreadContext(th, ctx))
                {
                    ulong rip = *(ulong*)((byte*)ctx + 0xF8);
                    // 待っているスレッドは同じ番地に居続ける: 歩き直さず前回のスタックを数える
                    if (lastRip.TryGetValue(kv.Key, out var lr) && lr.rip == rip) reuse = lr.key;
                    else lastRip[kv.Key] = (rip, null);
                    if (reuse == null) {
                    ulong rsp = *(ulong*)((byte*)ctx + 0x98);
                    ulong rbp = *(ulong*)((byte*)ctx + 0xA0);
                    new Span<byte>(frame, 512).Clear();
                    *(ulong*)(frame + 0) = rip; *(int*)(frame + 12) = 3;    // AddrPC, flat
                    *(ulong*)(frame + 32) = rbp; *(int*)(frame + 44) = 3;   // AddrFrame
                    *(ulong*)(frame + 48) = rsp; *(int*)(frame + 60) = 3;   // AddrStack
                    for (int depth = 0; depth < 96; depth++)
                    {
                        if (!StackWalk64(0x8664, hProc, th, frame, ctx, IntPtr.Zero, fta, gmb, IntPtr.Zero)) break;
                        ulong pc = *(ulong*)frame;
                        if (pc == 0) break;
                        addrs.Add(pc);
                    }
                    }
                }
                ResumeThread(th);
                if (reuse != null) { stacks[reuse] = stacks.GetValueOrDefault(reuse) + 1; continue; }
                if (addrs.Count == 0) continue;
                sb.Clear();
                for (int i = addrs.Count - 1; i >= 0; i--)
                {
                    // 戻り番地は呼び出しの次の命令を指すので、葉以外は 1 引いて呼び出し元の関数に入れる
                    ulong a = i == 0 ? addrs[i] : addrs[i] - 1;
                    if (sb.Length > 0) sb.Append(';');
                    sb.Append(Symbolize(a));
                }
                string key = sb.ToString();
                if (lastRip.TryGetValue(kv.Key, out var cur)) lastRip[kv.Key] = (cur.rip, key);
                stacks[key] = stacks.GetValueOrDefault(key) + 1;
                samples++;
            }
            Thread.Sleep(1);
        }
        if (timedOut)
        {
            Console.Error.WriteLine($"ABNORMAL TERMINATION: target exceeded {timeoutSec} s; killing the process tree");
            try { proc.Kill(true); } catch { }
        }
        proc.WaitForExit();
        File.WriteAllText(outPath + ".stdout", so.Result + "\n--- stderr ---\n" + se.Result);
        using (var w = new StreamWriter(outPath, false, new UTF8Encoding(false)))
            foreach (var kv in stacks.OrderByDescending(k => k.Value))
                w.WriteLine(kv.Key + " " + kv.Value);
        Console.WriteLine($"samples={samples} unique_stacks={stacks.Count} elapsed_ms={sw.ElapsedMilliseconds} exit={proc.ExitCode}");
        return timedOut ? 3 : 0;
    }
}
