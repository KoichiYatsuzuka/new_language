# git SHA: 08f19f554735e8588bc1f4bd2e2b300b43e4a31a
"""Built-in functions and collection method dispatch."""
from __future__ import annotations
from typing import Optional, Callable, TYPE_CHECKING

from .value import (
    Value, MISSING,
    TlList, TlDict, TlTuple, TlSet, TlFunction, TlOverloadedFn,
    TlGeneratorFn, TlGenerator, TlClass, TlInstance, TlType, TlTrait,
    TlNamespace, TlSlice, TlFileObject,
    type_name, display, is_truthy, _values_equal,
)
from .exceptions import RaiseSignal, StopIterationSignal

if TYPE_CHECKING:
    pass


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _make_error_instance(cls_name: str, message: str, known_classes: dict) -> "Value":
    """Build a language-level error instance for built-in exception types."""
    if cls_name in known_classes:
        cls = known_classes[cls_name]
        if isinstance(cls, TlClass):
            inst = TlInstance(cls=cls, fields={}, immutable=False)
            inst.fields["message"] = [message, False]
            inst.fields["code_context"] = ["", True]
            inst.fields["file"] = ["", True]
            inst.fields["line"] = [0, True]
            inst.fields["col"] = [0, True]
            return inst
    # Fallback: plain string
    return message


def _raise_builtin(name: str, msg: str, known_classes: dict) -> None:
    exc = _make_error_instance(name, msg, known_classes)
    raise RaiseSignal(exc, msg)


# ---------------------------------------------------------------------------
# Iteration protocol helpers
# ---------------------------------------------------------------------------

def iterate(val: Value) -> list[Value]:
    """Collect all items from an iterable value."""
    if isinstance(val, TlList):
        return list(val.items)
    if isinstance(val, TlTuple):
        return list(val.values)
    if isinstance(val, TlSet):
        return list(val.items)
    if isinstance(val, str):
        return list(val)
    if isinstance(val, TlDict):
        return list(val.keys)
    if isinstance(val, TlGenerator):
        items = val.values[val.index:]
        val.index = len(val.values)
        return items
    if isinstance(val, TlInstance):
        # Try __iter__ → collect items
        cls = val.cls
        if "__iter__" in cls.methods:
            return []  # handled by interpreter
    raise RuntimeError(f"TypeError: '{type_name(val)}' object is not iterable")


def gen_next(gen: TlGenerator) -> Value:
    if gen.index >= len(gen.values):
        raise StopIterationSignal()
    v = gen.values[gen.index]
    gen.index += 1
    return v


# ---------------------------------------------------------------------------
# Slice application
# ---------------------------------------------------------------------------

def apply_slice(obj: Value, sl: TlSlice, known_classes: dict) -> Value:
    def to_int(v: Optional[Value], default: int) -> int:
        if v is None:
            return default
        if isinstance(v, int) and not isinstance(v, bool):
            return v
        if isinstance(v, TlInstance) and v.cls.name in ("Index", "Size"):
            inner = v.fields.get("__value__")
            if inner:
                return int(inner[0])
        raise RuntimeError(f"TypeError: slice bounds must be int or Index, got '{type_name(v)}'")

    def to_step(v: Optional[Value]) -> int:
        if v is None:
            return 1
        if isinstance(v, int) and not isinstance(v, bool):
            return v
        raise RuntimeError(f"TypeError: slice step must be int, got '{type_name(v)}'")

    if isinstance(obj, (TlList, TlTuple, str)):
        seq = obj.items if isinstance(obj, TlList) else (obj.values if isinstance(obj, TlTuple) else list(obj))
        n = len(seq)

        begin_v, end_v, step_v = sl.begin, sl.end, sl.step
        step = to_step(step_v)
        if step == 0:
            raise RuntimeError("ValueError: slice step cannot be zero")

        if step > 0:
            begin = to_int(begin_v, 0)
            end = to_int(end_v, n)
        else:
            begin = to_int(begin_v, n - 1)
            end = to_int(end_v, -(n + 1))

        # Python-compatible negative index normalization
        def norm(i: int, length: int, default: int) -> int:
            if i < 0:
                i = i + length
            return i

        if step > 0:
            begin = max(0, norm(begin, n, 0))
            end = min(n, norm(end, n, n))
            result = seq[begin:end:step]
        else:
            begin_n = norm(begin, n, n - 1) if begin_v is not None else n - 1
            end_n = norm(end, n, -(n + 1)) if end_v is not None else -(n + 1)
            begin_n = min(n - 1, max(-1, begin_n))
            end_n = max(-(n + 1), min(n - 1, end_n))
            result = []
            i = begin_n
            while i > end_n and i >= 0:
                result.append(seq[i])
                i += step

        if isinstance(obj, TlList):
            return TlList(items=result)
        if isinstance(obj, TlTuple):
            return TlTuple(values=result)
        return "".join(result)  # str

    raise RuntimeError(f"TypeError: '{type_name(obj)}' object is not sliceable")


# ---------------------------------------------------------------------------
# subscript (index access)
# ---------------------------------------------------------------------------

def subscript_get(obj: Value, index: Value, known_classes: dict) -> Value:
    if isinstance(index, TlSlice):
        return apply_slice(obj, index, known_classes)

    if isinstance(obj, TlList):
        i = _to_index(index, len(obj.items), known_classes)
        return obj.items[i]

    if isinstance(obj, TlTuple):
        i = _to_index(index, len(obj.values), known_classes)
        return obj.values[i]

    if isinstance(obj, str):
        i = _to_index(index, len(obj), known_classes)
        return obj[i]

    if isinstance(obj, TlDict):
        val = obj.get(index)
        if val is MISSING:
            key_str = display(index)
            raise RaiseSignal(
                _make_error_instance("KeyError", f"key not found: {key_str}", known_classes),
                f"KeyError: key not found: {key_str}",
            )
        return val  # type: ignore[return-value]

    if isinstance(obj, TlInstance):
        # Check for __getitem__
        cls = obj.cls
        if "__getitem__" in cls.methods:
            return None  # let interpreter handle method call
        raise RuntimeError(f"TypeError: '{type_name(obj)}' object is not subscriptable")

    raise RuntimeError(f"TypeError: '{type_name(obj)}' object is not subscriptable")


def subscript_set(obj: Value, index: Value, value: Value, known_classes: dict) -> None:
    if isinstance(obj, TlList):
        if isinstance(index, TlSlice):
            # Slice assignment
            n = len(obj.items)
            def to_int_s(v: Optional[Value], default: int) -> int:
                if v is None: return default
                if isinstance(v, int) and not isinstance(v, bool): return v
                if isinstance(v, TlInstance) and v.cls.name in ("Index", "Size"):
                    inner = v.fields.get("__value__")
                    if inner: return int(inner[0])
                return default
            begin = to_int_s(index.begin, 0)
            end = to_int_s(index.end, n)
            if begin < 0: begin = max(0, begin + n)
            if end < 0: end = max(0, end + n)
            begin = max(0, min(n, begin))
            end = max(0, min(n, end))
            new_items = value.items if isinstance(value, TlList) else [value]
            obj.items[begin:end] = new_items
            return
        i = _to_index(index, len(obj.items), known_classes)
        obj.items[i] = value
        return
    if isinstance(obj, TlDict):
        obj.set(index, value)
        return
    if isinstance(obj, TlInstance):
        cls = obj.cls
        if "__setitem__" in cls.methods:
            return  # let interpreter handle
    raise RuntimeError(f"TypeError: '{type_name(obj)}' object does not support item assignment")


def _to_index(index: Value, length: int, known_classes: dict) -> int:
    if isinstance(index, int) and not isinstance(index, bool):
        i = index
        if i < 0:
            i += length
        if not (0 <= i < length):
            raise RaiseSignal(
                _make_error_instance("IndexError", f"index {index} out of range", known_classes),
                f"IndexError: index {index} out of range",
            )
        return i
    if isinstance(index, TlInstance) and index.cls.name in ("Index", "Size"):
        inner = index.fields.get("__value__")
        if inner:
            return _to_index(inner[0], length, known_classes)
    raise RuntimeError(f"TypeError: indices must be integers, got '{type_name(index)}'")


# ---------------------------------------------------------------------------
# Attribute access on built-in types
# ---------------------------------------------------------------------------

def get_attr_builtin(obj: Value, attr: str, known_classes: dict) -> Value:
    """Return a bound method or attribute for built-in types."""

    # ---- list methods ----
    if isinstance(obj, TlList):
        items = obj.items
        def _bound(fn: Callable) -> Value:
            return TlFunction(name=attr, params=[], body=[], captured_env={"__self__": None},  # type: ignore
                              is_static=False)
        if attr == "append":
            def append(args, kwargs): items.append(args[0]); return None
            return _make_native(attr, append)
        if attr == "pop":
            def pop(args, kwargs):
                if args:
                    i = args[0]
                    if isinstance(i, int): return items.pop(i)
                if not items:
                    raise RuntimeError("IndexError: pop from empty list")
                return items.pop()
            return _make_native(attr, pop)
        if attr == "insert":
            def insert(args, kwargs): items.insert(int(args[0]), args[1]); return None
            return _make_native(attr, insert)
        if attr == "remove":
            def remove(args, kwargs):
                v = args[0]
                for i, x in enumerate(items):
                    if _values_equal(x, v):
                        del items[i]; return None
                _raise_builtin("ValueError", f"list.remove(x): x not in list", known_classes)
            return _make_native(attr, remove)
        if attr == "clear":
            def clear(args, kwargs): items.clear(); return None
            return _make_native(attr, clear)
        if attr == "copy":
            def copy(args, kwargs): return TlList(items=list(items))
            return _make_native(attr, copy)
        if attr == "extend":
            def extend(args, kwargs):
                other = args[0]
                if isinstance(other, TlList): items.extend(other.items)
                elif isinstance(other, TlTuple): items.extend(other.values)
                return None
            return _make_native(attr, extend)
        if attr == "index":
            def index(args, kwargs):
                v = args[0]
                for i, x in enumerate(items):
                    if _values_equal(x, v): return i
                _raise_builtin("ValueError", f"value not in list", known_classes)
            return _make_native(attr, index)
        if attr == "count":
            def count(args, kwargs):
                return sum(1 for x in items if _values_equal(x, args[0]))
            return _make_native(attr, count)
        if attr == "reverse":
            def reverse(args, kwargs): items.reverse(); return None
            return _make_native(attr, reverse)
        if attr == "sort":
            def sort(args, kwargs):
                try:
                    items.sort(key=lambda v: v if isinstance(v, (int, float, str)) else 0)
                except TypeError:
                    pass
                return None
            return _make_native(attr, sort)
        if attr == "__iter__":
            def list_iter(args, kwargs):
                return TlGenerator(values=list(items))
            return _make_native(attr, list_iter)
        raise RuntimeError(f"AttributeError: 'list' object has no attribute '{attr}'")

    # ---- str methods ----
    if isinstance(obj, str):
        if attr == "upper": return _make_native(attr, lambda a, k: obj.upper())
        if attr == "lower": return _make_native(attr, lambda a, k: obj.lower())
        if attr == "strip": return _make_native(attr, lambda a, k: obj.strip())
        if attr == "lstrip": return _make_native(attr, lambda a, k: obj.lstrip())
        if attr == "rstrip": return _make_native(attr, lambda a, k: obj.rstrip())
        if attr == "split":
            def split(args, kwargs):
                sep = args[0] if args else None
                if sep is None:
                    parts = obj.split()
                else:
                    parts = obj.split(sep)
                return TlList(items=parts)
            return _make_native(attr, split)
        if attr == "join":
            def join(args, kwargs):
                seq = args[0]
                if isinstance(seq, TlList):
                    return obj.join(display(x) for x in seq.items)
                if isinstance(seq, TlTuple):
                    return obj.join(display(x) for x in seq.values)
                raise RuntimeError("TypeError: join argument must be list or tuple")
            return _make_native(attr, join)
        if attr == "startswith":
            return _make_native(attr, lambda a, k: obj.startswith(a[0]))
        if attr == "endswith":
            return _make_native(attr, lambda a, k: obj.endswith(a[0]))
        if attr == "find":
            return _make_native(attr, lambda a, k: obj.find(a[0]))
        if attr == "replace":
            return _make_native(attr, lambda a, k: obj.replace(a[0], a[1]))
        if attr == "format":
            def fmt(args, kwargs):
                try:
                    py_args = [display(a) for a in args]
                    py_kwargs = {k: display(v) for k, v in kwargs.items()}
                    return obj.format(*py_args, **py_kwargs)
                except Exception as e:
                    raise RuntimeError(f"ValueError: format error: {e}")
            return _make_native(attr, fmt)
        if attr == "count":
            return _make_native(attr, lambda a, k: obj.count(a[0]))
        if attr == "contains":
            return _make_native(attr, lambda a, k: a[0] in obj)
        if attr == "isdigit":
            return _make_native(attr, lambda a, k: obj.isdigit())
        if attr == "isalpha":
            return _make_native(attr, lambda a, k: obj.isalpha())
        if attr == "isalnum":
            return _make_native(attr, lambda a, k: obj.isalnum())
        raise RuntimeError(f"AttributeError: 'str' object has no attribute '{attr}'")

    # ---- dict methods ----
    if isinstance(obj, TlDict):
        if attr == "keys":
            return _make_native(attr, lambda a, k: TlList(items=list(obj.keys)))
        if attr == "values":
            return _make_native(attr, lambda a, k: TlList(items=list(obj.values)))
        if attr == "items":
            def items_fn(a, k):
                pairs = [TlTuple(values=[ki, vi]) for ki, vi in zip(obj.keys, obj.values)]
                return TlList(items=pairs)
            return _make_native(attr, items_fn)
        if attr == "get":
            def dict_get(args, kwargs):
                v = obj.get(args[0])
                if v is MISSING:
                    return args[1] if len(args) > 1 else None
                return v
            return _make_native(attr, dict_get)
        if attr == "pop":
            def dict_pop(args, kwargs):
                key = args[0]
                v = obj.get(key)
                if v is MISSING:
                    if len(args) > 1: return args[1]
                    _raise_builtin("KeyError", f"key not found: {display(key)}", known_classes)
                obj.remove(key)
                return v
            return _make_native(attr, dict_pop)
        if attr == "clear":
            def dict_clear(args, kwargs):
                obj.keys.clear(); obj.values.clear(); return None
            return _make_native(attr, dict_clear)
        if attr == "update":
            def dict_update(args, kwargs):
                other = args[0]
                if isinstance(other, TlDict):
                    for k, v in zip(other.keys, other.values):
                        obj.set(k, v)
                return None
            return _make_native(attr, dict_update)
        if attr == "contains":
            return _make_native(attr, lambda a, k: obj.get(a[0]) is not MISSING)
        raise RuntimeError(f"AttributeError: 'dict' object has no attribute '{attr}'")

    # ---- set methods ----
    if isinstance(obj, TlSet):
        if attr == "add":
            return _make_native(attr, lambda a, k: (obj.add(a[0]), None)[1])
        if attr == "discard":
            return _make_native(attr, lambda a, k: (obj.discard(a[0]), None)[1])
        if attr == "remove":
            def set_remove(args, kwargs):
                if not obj.remove(args[0]):
                    _raise_builtin("KeyError", f"'{display(args[0])}' not in set", known_classes)
                return None
            return _make_native(attr, set_remove)
        if attr == "pop":
            def set_pop(args, kwargs):
                if not obj.items:
                    _raise_builtin("KeyError", "pop from an empty set", known_classes)
                return obj.items.pop()
            return _make_native(attr, set_pop)
        if attr == "clear":
            return _make_native(attr, lambda a, k: (obj.items.clear(), None)[1])
        if attr == "copy":
            return _make_native(attr, lambda a, k: TlSet(items=list(obj.items)))
        if attr == "union":
            def union(args, kwargs):
                other = args[0]
                new = TlSet(items=list(obj.items))
                for x in (other.items if isinstance(other, TlSet) else []):
                    new.add(x)
                return new
            return _make_native(attr, union)
        if attr == "intersection":
            def intersect(args, kwargs):
                other = args[0]
                other_items = other.items if isinstance(other, TlSet) else []
                return TlSet(items=[x for x in obj.items if any(_values_equal(x, y) for y in other_items)])
            return _make_native(attr, intersect)
        if attr == "difference":
            def diff(args, kwargs):
                other = args[0]
                other_items = other.items if isinstance(other, TlSet) else []
                return TlSet(items=[x for x in obj.items if not any(_values_equal(x, y) for y in other_items)])
            return _make_native(attr, diff)
        if attr == "symmetric_difference":
            def sym_diff(args, kwargs):
                other = args[0]
                other_items = other.items if isinstance(other, TlSet) else []
                in_self_not_other = [x for x in obj.items if not any(_values_equal(x, y) for y in other_items)]
                in_other_not_self = [y for y in other_items if not any(_values_equal(x, y) for x in obj.items)]
                return TlSet(items=in_self_not_other + in_other_not_self)
            return _make_native(attr, sym_diff)
        if attr == "issubset":
            def issubset(args, kwargs):
                other = args[0]
                other_items = other.items if isinstance(other, TlSet) else []
                return all(any(_values_equal(x, y) for y in other_items) for x in obj.items)
            return _make_native(attr, issubset)
        if attr == "issuperset":
            def issuperset(args, kwargs):
                other = args[0]
                other_items = other.items if isinstance(other, TlSet) else []
                return all(any(_values_equal(x, y) for x in obj.items) for y in other_items)
            return _make_native(attr, issuperset)
        if attr == "contains":
            return _make_native(attr, lambda a, k: obj.contains(a[0]))
        raise RuntimeError(f"AttributeError: 'set' object has no attribute '{attr}'")

    # ---- tuple methods ----
    if isinstance(obj, TlTuple):
        if attr == "count":
            return _make_native(attr, lambda a, k: sum(1 for x in obj.values if _values_equal(x, a[0])))
        if attr == "index":
            def tup_index(args, kwargs):
                for i, x in enumerate(obj.values):
                    if _values_equal(x, args[0]): return i
                _raise_builtin("ValueError", "tuple.index(x): x not in tuple", known_classes)
            return _make_native(attr, tup_index)
        raise RuntimeError(f"AttributeError: 'tuple' object has no attribute '{attr}'")

    # ---- slice attributes ----
    if isinstance(obj, TlSlice):
        if attr == "begin": return obj.begin
        if attr == "end": return obj.end
        if attr == "step": return obj.step
        raise RuntimeError(f"AttributeError: 'slice' object has no attribute '{attr}'")

    # ---- generator ----
    if isinstance(obj, TlGenerator):
        if attr == "next":
            def gen_next_fn(args, kwargs):
                from .exceptions import StopIterationSignal
                try:
                    return gen_next(obj)
                except StopIterationSignal:
                    _raise_builtin("StopIteration", "generator exhausted", known_classes)
            return _make_native(attr, gen_next_fn)
        raise RuntimeError(f"AttributeError: 'generator' object has no attribute '{attr}'")

    # ---- file methods ----
    if isinstance(obj, TlFileObject):
        if obj.is_closed:
            raise RuntimeError("ValueError: I/O operation on closed file")
        if attr == "read":
            def file_read(args, kwargs):
                if obj.text_mode:
                    return obj.content[obj.pointer:].decode("utf-8", errors="replace")
                else:
                    return TlList(items=list(obj.content[obj.pointer:]))
            return _make_native(attr, file_read)
        if attr == "read_line":
            def read_line(args, kwargs):
                data = obj.content[obj.pointer:]
                nl = data.find(b"\n")
                if nl == -1:
                    line = data
                    obj.pointer = len(obj.content)
                else:
                    line = data[:nl + 1]
                    obj.pointer += nl + 1
                return line.decode("utf-8", errors="replace") if obj.text_mode else TlList(items=list(line))
            return _make_native(attr, read_line)
        if attr == "write":
            def file_write(args, kwargs):
                s = args[0]
                if isinstance(s, str):
                    b = s.encode("utf-8")
                elif isinstance(s, TlList):
                    b = bytes(int(x) for x in s.items)
                else:
                    raise RuntimeError("TypeError: write argument must be str or list[int]")
                if obj.pointer < len(obj.content):
                    obj.content[obj.pointer:obj.pointer + len(b)] = b
                else:
                    obj.content.extend(b)
                obj.pointer += len(b)
                return None
            return _make_native(attr, file_write)
        if attr == "write_line":
            def write_line(args, kwargs):
                s = args[0] if args else ""
                if isinstance(s, str):
                    b = (s + "\n").encode("utf-8")
                else:
                    raise RuntimeError("TypeError: write_line argument must be str")
                obj.content.extend(b)
                obj.pointer += len(b)
                return None
            return _make_native(attr, write_line)
        if attr == "close":
            def file_close(args, kwargs):
                obj.close(); return None
            return _make_native(attr, file_close)
        if attr == "read_letter":
            def read_letter(args, kwargs):
                if obj.pointer >= len(obj.content):
                    return ""
                ch = obj.content[obj.pointer:obj.pointer+1]
                obj.pointer += 1
                return ch.decode("utf-8", errors="replace") if obj.text_mode else ch[0]
            return _make_native(attr, read_letter)
        if attr == "seek":
            def seek(args, kwargs):
                obj.pointer = int(args[0]) if args else 0
                return None
            return _make_native(attr, seek)
        raise RuntimeError(f"AttributeError: 'file' object has no attribute '{attr}'")

    raise RuntimeError(f"AttributeError: '{type_name(obj)}' object has no attribute '{attr}'")


# ---------------------------------------------------------------------------
# Native callable shim
# ---------------------------------------------------------------------------

class _NativeCallable:
    """Wraps a Python callable so the interpreter can call it uniformly."""
    def __init__(self, name: str, fn: Callable) -> None:
        self.name = name
        self._fn = fn

    def call(self, args: list[Value], kwargs: dict[str, Value]) -> Value:
        return self._fn(args, kwargs)

    def __repr__(self) -> str:
        return f"<built-in function {self.name}>"


def _make_native(name: str, fn: Callable) -> _NativeCallable:
    return _NativeCallable(name, fn)


# ---------------------------------------------------------------------------
# Built-in function table
# ---------------------------------------------------------------------------

def make_builtins(known_classes: dict) -> dict[str, Value]:
    """Return the global built-in name→value map."""

    def builtin_print(args: list, kwargs: dict) -> None:
        sep = kwargs.get("sep", " ")
        end = kwargs.get("end", "\n")
        if not isinstance(sep, str): sep = " "
        if not isinstance(end, str): end = "\n"
        print(sep.join(display(a) for a in args), end=end)
        return None

    def builtin_len(args: list, kwargs: dict) -> int:
        v = args[0]
        if isinstance(v, TlList): return len(v.items)
        if isinstance(v, TlDict): return len(v.keys)
        if isinstance(v, TlTuple): return len(v.values)
        if isinstance(v, TlSet): return len(v.items)
        if isinstance(v, str): return len(v)
        raise RuntimeError(f"TypeError: len() not supported for '{type_name(v)}'")

    def builtin_range(args: list, kwargs: dict) -> TlList:
        if len(args) == 1:
            start, stop, step = 0, int(args[0]), 1
        elif len(args) == 2:
            start, stop, step = int(args[0]), int(args[1]), 1
        else:
            start, stop, step = int(args[0]), int(args[1]), int(args[2])
        return TlList(items=list(range(start, stop, step)))

    def builtin_enumerate(args: list, kwargs: dict) -> TlList:
        iterable = args[0]
        start = int(args[1]) if len(args) > 1 else 0
        items = iterate(iterable)
        return TlList(items=[TlTuple(values=[i + start, x]) for i, x in enumerate(items)])

    def builtin_zip(args: list, kwargs: dict) -> TlList:
        lists = [iterate(a) for a in args]
        return TlList(items=[TlTuple(values=list(row)) for row in zip(*lists)])

    def builtin_type(args: list, kwargs: dict) -> str:
        return type_name(args[0])

    def builtin_id_raw(args: list, kwargs: dict) -> int:
        return id(args[0]) if args else 0

    def builtin_str(args: list, kwargs: dict) -> str:
        if not args: return ""
        return display(args[0])

    def builtin_repr(args: list, kwargs: dict) -> str:
        from .value import _repr_val
        return _repr_val(args[0])

    def builtin_int(args: list, kwargs: dict) -> int:
        v = args[0]
        if isinstance(v, bool): return int(v)
        if isinstance(v, int): return v
        if isinstance(v, float): return int(v)
        if isinstance(v, str):
            try: return int(v)
            except ValueError: raise RuntimeError(f"ValueError: invalid literal for int(): '{v}'")
        raise RuntimeError(f"TypeError: int() argument must be int, float or str, not '{type_name(v)}'")

    def builtin_float(args: list, kwargs: dict) -> float:
        v = args[0]
        if isinstance(v, (int, float)) and not isinstance(v, bool): return float(v)
        if isinstance(v, bool): return float(v)
        if isinstance(v, str):
            try: return float(v)
            except ValueError: raise RuntimeError(f"ValueError: invalid literal for float(): '{v}'")
        raise RuntimeError(f"TypeError: float() argument must be numeric or str, not '{type_name(v)}'")

    def builtin_bool(args: list, kwargs: dict) -> bool:
        return is_truthy(args[0]) if args else False

    def builtin_abs(args: list, kwargs: dict) -> Value:
        v = args[0]
        if isinstance(v, int) and not isinstance(v, bool): return abs(v)
        if isinstance(v, float): return abs(v)
        raise RuntimeError(f"TypeError: abs() not supported for '{type_name(v)}'")

    def builtin_min(args: list, kwargs: dict) -> Value:
        if len(args) == 1 and isinstance(args[0], TlList):
            items = args[0].items
        else:
            items = args
        if not items: raise RuntimeError("ValueError: min() arg is an empty sequence")
        result = items[0]
        for x in items[1:]:
            if _cmp_lt(x, result): result = x
        return result

    def builtin_max(args: list, kwargs: dict) -> Value:
        if len(args) == 1 and isinstance(args[0], TlList):
            items = args[0].items
        else:
            items = args
        if not items: raise RuntimeError("ValueError: max() arg is an empty sequence")
        result = items[0]
        for x in items[1:]:
            if _cmp_lt(result, x): result = x
        return result

    def builtin_sum(args: list, kwargs: dict) -> Value:
        iterable = args[0]
        items = iterate(iterable)
        start: Value = args[1] if len(args) > 1 else 0
        result = start
        for x in items:
            if isinstance(result, int) and isinstance(x, int): result = result + x
            elif isinstance(result, (int, float)) and isinstance(x, (int, float)): result = float(result) + float(x)
            else: raise RuntimeError(f"TypeError: sum() not supported for '{type_name(x)}'")
        return result

    def builtin_sorted(args: list, kwargs: dict) -> TlList:
        iterable = args[0]
        items = list(iterate(iterable))
        reverse = is_truthy(kwargs.get("reverse", False))
        try:
            items.sort(key=lambda v: v if isinstance(v, (int, float, str)) else 0, reverse=reverse)
        except TypeError:
            pass
        return TlList(items=items)

    def builtin_reversed(args: list, kwargs: dict) -> TlList:
        items = list(iterate(args[0]))
        return TlList(items=list(reversed(items)))

    def builtin_list(args: list, kwargs: dict) -> TlList:
        if not args: return TlList(items=[])
        items = iterate(args[0])
        return TlList(items=items)

    def builtin_dict(args: list, kwargs: dict) -> TlDict:
        d = TlDict()
        if args and isinstance(args[0], TlList):
            for item in args[0].items:
                if isinstance(item, TlTuple) and len(item.values) == 2:
                    d.set(item.values[0], item.values[1])
        return d

    def builtin_tuple(args: list, kwargs: dict) -> TlTuple:
        if not args: return TlTuple(values=[])
        items = iterate(args[0])
        return TlTuple(values=items)

    def builtin_set(args: list, kwargs: dict) -> TlSet:
        if not args: return TlSet(items=[])
        items = iterate(args[0])
        s = TlSet(items=[])
        for x in items: s.add(x)
        return s

    def builtin_slice(args: list, kwargs: dict) -> TlSlice:
        if len(args) == 1:
            return TlSlice(begin=None, end=args[0], step=None)
        if len(args) == 2:
            return TlSlice(begin=args[0], end=args[1], step=None)
        return TlSlice(begin=args[0], end=args[1], step=args[2])

    def builtin_isinstance(args: list, kwargs: dict) -> bool:
        obj, tp = args[0], args[1]
        if isinstance(tp, TlType):
            tn = tp.name
        elif isinstance(tp, str):
            tn = tp
        else:
            return False
        return type_name(obj) == tn

    def builtin_hasattr(args: list, kwargs: dict) -> bool:
        obj, name = args[0], args[1]
        if not isinstance(name, str): return False
        if isinstance(obj, TlInstance):
            return name in obj.fields or name in obj.cls.methods
        return False

    def builtin_getattr(args: list, kwargs: dict) -> Value:
        obj, name = args[0], args[1]
        default = args[2] if len(args) > 2 else MISSING
        if isinstance(obj, TlInstance):
            if name in obj.fields:
                return obj.fields[name][0]
            if name in obj.cls.methods:
                return obj.cls.methods[name][0]
        if default is not MISSING:
            return default  # type: ignore[return-value]
        raise RuntimeError(f"AttributeError: '{type_name(obj)}' object has no attribute '{name}'")

    def builtin_input(args: list, kwargs: dict) -> str:
        prompt = display(args[0]) if args else ""
        return input(prompt)

    def builtin_open(args: list, kwargs: dict) -> TlFileObject:
        import os
        fpath = args[0]
        if isinstance(fpath, TlInstance) and fpath.cls.new_type_base == "str":
            fpath = fpath.fields.get("__value__", [fpath])[0]
        if not isinstance(fpath, str):
            raise RuntimeError("TypeError: open() path must be str or path")

        # Mode: 2nd positional arg or "mode" kwarg; can be str or enum instance
        raw_mode = args[1] if len(args) > 1 else kwargs.get("mode", "r")

        # Map FileOpenMode enum value to internal mode string
        _enum_to_mode = {0: "w", 1: "rw_trunc", 2: "r", 3: "rw_new", 4: "rw"}
        if isinstance(raw_mode, TlInstance):
            vf = raw_mode.fields.get("value")
            iv = vf[0] if vf else 2
            mode_name = _enum_to_mode.get(iv, "r")
        elif isinstance(raw_mode, str):
            mode_name = raw_mode
        else:
            mode_name = "r"

        # StartPoint: 3rd arg
        start_val = 0
        if len(args) > 2 and isinstance(args[2], TlInstance):
            vf = args[2].fields.get("value")
            start_val = vf[0] if vf else 0

        # ByteRecognizingMode: 4th arg
        text_mode = True
        if len(args) > 3 and isinstance(args[3], TlInstance):
            vf = args[3].fields.get("value")
            bm = vf[0] if vf else 1
            text_mode = (bm == 1)

        if mode_name == "r":
            if not os.path.exists(fpath):
                raise RaiseSignal(
                    _make_error_instance("FileNotFoundError", f"No such file: '{fpath}'", known_classes),
                    f"FileNotFoundError: No such file: '{fpath}'",
                )
            content = bytearray(open(fpath, "rb").read())
            pointer = len(content) if start_val == 2 else 0
            return TlFileObject(path=fpath, mode="r", content=content, pointer=pointer, text_mode=text_mode)
        if mode_name in ("w", "rw_trunc"):
            content = bytearray()
            return TlFileObject(path=fpath, mode="w", content=content, text_mode=text_mode)
        if mode_name in ("rw", "rw+"):
            content = bytearray(open(fpath, "rb").read()) if os.path.exists(fpath) else bytearray()
            return TlFileObject(path=fpath, mode="rw", content=content, text_mode=text_mode)
        if mode_name == "rw_new":
            content = bytearray()
            return TlFileObject(path=fpath, mode="rw_new", content=content, text_mode=text_mode)
        raise RuntimeError(f"ValueError: unsupported open mode '{mode_name}'")

    def builtin_path(args: list, kwargs: dict) -> Value:
        """path('some/path') → new_type path instance wrapping a string."""
        s = args[0] if args else ""
        if not isinstance(s, str): s = display(s)
        return TlInstance(cls=path_cls, fields={"__value__": [s, False]}, immutable=False)

    def builtin_chr(args: list, kwargs: dict) -> str:
        return chr(int(args[0]))

    def builtin_ord(args: list, kwargs: dict) -> int:
        s = args[0]
        if isinstance(s, str) and len(s) == 1:
            return ord(s)
        raise RuntimeError(f"TypeError: ord() expects a single character, got '{s}'")

    def builtin_hex(args: list, kwargs: dict) -> str:
        return hex(int(args[0]))

    def builtin_oct(args: list, kwargs: dict) -> str:
        return oct(int(args[0]))

    def builtin_bin(args: list, kwargs: dict) -> str:
        return bin(int(args[0]))

    def builtin_format(args: list, kwargs: dict) -> str:
        val, spec = args[0], args[1] if len(args) > 1 else ""
        if isinstance(spec, str):
            try:
                return format(val if not isinstance(val, bool) else int(val), spec)
            except Exception:
                return display(val)
        return display(val)

    def builtin_any(args: list, kwargs: dict) -> bool:
        return any(is_truthy(x) for x in iterate(args[0]))

    def builtin_all(args: list, kwargs: dict) -> bool:
        return all(is_truthy(x) for x in iterate(args[0]))

    def builtin_close(args: list, kwargs: dict) -> None:
        f = args[0]
        if isinstance(f, TlFileObject):
            f.close()
        return None

    def builtin_map(args: list, kwargs: dict) -> TlList:
        # Limited map — callable must be a TlFunction
        fn, iterable = args[0], args[1]
        items = iterate(iterable)
        # Return a list with a marker; actual call happens in interpreter
        return TlList(items=items)  # placeholder

    def builtin_filter(args: list, kwargs: dict) -> TlList:
        iterable = args[1] if len(args) > 1 else args[0]
        return TlList(items=list(iterate(iterable)))  # placeholder

    # Build built-in new_type classes (Index, Size)
    def _make_new_type_cls(name: str, base: str = "int") -> TlClass:
        cls = TlClass(
            name=name, bases=[], methods={}, gen_methods={},
            field_defaults=[("__value__", None, False)],
            class_vars={}, field_mutability={"__value__": False},
            field_access={}, method_access={},
            static_method_names=set(), class_method_names=set(),
            static_vars={}, new_type_base=base,
        )
        known_classes[name] = cls
        return cls

    index_cls = _make_new_type_cls("Index")
    size_cls = _make_new_type_cls("Size")
    uint_cls = _make_new_type_cls("uint")
    pointer_cls = _make_new_type_cls("pointer")
    path_cls = _make_new_type_cls("path", base="str")

    def _make_index_instance(cls: TlClass, args, kwargs):
        val = args[0] if args else 0
        if isinstance(val, bool): val = int(val)
        inst = TlInstance(cls=cls, fields={"__value__": [val, False]}, immutable=False)
        return inst

    def builtin_Index(args, kwargs):
        return _make_index_instance(index_cls, args, kwargs)

    def builtin_Size(args, kwargs):
        return _make_index_instance(size_cls, args, kwargs)

    def builtin_uint(args, kwargs):
        val = int(args[0]) if args else 0
        return TlInstance(cls=uint_cls, fields={"__value__": [val, False]}, immutable=False)

    def builtin_pointer(args, kwargs):
        val = int(args[0]) if args else 0
        return TlInstance(cls=pointer_cls, fields={"__value__": [val, False]}, immutable=False)

    def builtin_id(args, kwargs):
        raw = id(args[0]) if args else 0
        return TlInstance(cls=pointer_cls, fields={"__value__": [raw, False]}, immutable=False)

    builtins: dict[str, Value] = {
        "print":     _make_native("print", builtin_print),
        "len":       _make_native("len", builtin_len),
        "range":     _make_native("range", builtin_range),
        "enumerate": _make_native("enumerate", builtin_enumerate),
        "zip":       _make_native("zip", builtin_zip),
        "type":      _make_native("type", builtin_type),
        "id":        _make_native("id", builtin_id),  # id() returns pointer instance
        "str":       _make_native("str", builtin_str),
        "repr":      _make_native("repr", builtin_repr),
        "int":       _make_native("int", builtin_int),
        "float":     _make_native("float", builtin_float),
        "bool":      _make_native("bool", builtin_bool),
        "abs":       _make_native("abs", builtin_abs),
        "min":       _make_native("min", builtin_min),
        "max":       _make_native("max", builtin_max),
        "sum":       _make_native("sum", builtin_sum),
        "sorted":    _make_native("sorted", builtin_sorted),
        "reversed":  _make_native("reversed", builtin_reversed),
        "list":      _make_native("list", builtin_list),
        "dict":      _make_native("dict", builtin_dict),
        "tuple":     _make_native("tuple", builtin_tuple),
        "set":       _make_native("set", builtin_set),
        "slice":     _make_native("slice", builtin_slice),
        "isinstance": _make_native("isinstance", builtin_isinstance),
        "hasattr":   _make_native("hasattr", builtin_hasattr),
        "getattr":   _make_native("getattr", builtin_getattr),
        "input":     _make_native("input", builtin_input),
        "open":      _make_native("open", builtin_open),
        "path":      _make_native("path", builtin_path),
        "chr":       _make_native("chr", builtin_chr),
        "ord":       _make_native("ord", builtin_ord),
        "hex":       _make_native("hex", builtin_hex),
        "oct":       _make_native("oct", builtin_oct),
        "bin":       _make_native("bin", builtin_bin),
        "format":    _make_native("format", builtin_format),
        "any":       _make_native("any", builtin_any),
        "all":       _make_native("all", builtin_all),
        "close":     _make_native("close", builtin_close),
        "Index":     _make_native("Index", builtin_Index),
        "Size":      _make_native("Size", builtin_Size),
        "uint":      _make_native("uint", builtin_uint),
        "pointer":   _make_native("pointer", builtin_pointer),
        # Type values
        "int_type":  TlType("int"),
        "str_type":  TlType("str"),
        "float_type": TlType("float"),
        "bool_type": TlType("bool"),
        "None":      None,
        "True":      True,
        "False":     False,
    }
    return builtins


def _cmp_lt(a: Value, b: Value) -> bool:
    if isinstance(a, bool) or isinstance(b, bool):
        return False
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return float(a) < float(b)
    if isinstance(a, str) and isinstance(b, str):
        return a < b
    return False
