using System.Collections.Generic;
using System.Threading;

namespace FormBridge;

/// <summary>Handle table shared across all bridge exports.</summary>
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
            throw new System.InvalidOperationException(
                $"Invalid handle {id} for type {typeof(T).Name}");
        }
    }

    internal static void Release(long id)
    {
        lock (_table) { _table.Remove(id); }
    }
}
