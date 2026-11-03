# sys.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/sys/__init__.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import MaybeNone, OptExcInfo, ProfileFunction, StrOrBytesPath, TraceFunction, structseq
from _typeshed.importlib import MetaPathFinderProtocol, PathEntryFinderProtocol
from builtins import object as _object
from collections.abc import AsyncGenerator, Callable, Sequence
from io import TextIOWrapper
from types import FrameType, ModuleType, SimpleNamespace, TracebackType
from typing import Any, Final, Literal, NoReturn, Protocol, TextIO, TypeAlias, TypeVar, final, overload, type_check_only
from typing_extensions import LiteralString, deprecated
argv: list[str]
base_exec_prefix: str
base_prefix: str
byteorder: Literal['little', 'big']
builtin_module_names: Sequence[str]
copyright: str
dllhandle: int
dont_write_bytecode: bool
displayhook: Callable[[object], Any]
excepthook: Callable[[type[BaseException], BaseException, TracebackType | None], Any]
exec_prefix: str
executable: str
float_repr_style: Literal['short', 'legacy']
hexversion: int
last_type: type[BaseException] | None
last_value: BaseException | None
last_traceback: TracebackType | None
last_exc: BaseException
maxsize: int
maxunicode: int
meta_path: list[MetaPathFinderProtocol]
modules: dict[str, ModuleType]
orig_argv: list[str]
path: list[str]
path_hooks: list[Callable[[str], PathEntryFinderProtocol]]
path_importer_cache: dict[str, PathEntryFinderProtocol | None]
platform: LiteralString
platlibdir: str
prefix: str
pycache_prefix: str | None
ps1: object
ps2: object
stdin: TextIO | MaybeNone
stdout: TextIO | MaybeNone
stderr: TextIO | MaybeNone
stdlib_module_names: frozenset[str]
__stdin__: Final[TextIOWrapper | None]
__stdout__: Final[TextIOWrapper | None]
__stderr__: Final[TextIOWrapper | None]
tracebacklimit: int | None
version: str
api_version: int
warnoptions: Any
winver: str
_xoptions: dict[Any, Any]
flags: _flags

@final
@type_check_only
class _flags(_UninstantiableStructseq, tuple[int, ...]):
    __match_args__: Final = ('debug', 'inspect', 'interactive', 'optimize', 'dont_write_bytecode', 'no_user_site', 'no_site', 'ignore_environment', 'verbose', 'bytes_warning', 'quiet', 'hash_randomization', 'isolated', 'dev_mode', 'utf8_mode', 'warn_default_encoding', 'safe_path', 'int_max_str_digits')

    @property
    def debug(self) -> int:
        ...

    @property
    def inspect(self) -> int:
        ...

    @property
    def interactive(self) -> int:
        ...

    @property
    def optimize(self) -> int:
        ...

    @property
    def dont_write_bytecode(self) -> int:
        ...

    @property
    def no_user_site(self) -> int:
        ...

    @property
    def no_site(self) -> int:
        ...

    @property
    def ignore_environment(self) -> int:
        ...

    @property
    def verbose(self) -> int:
        ...

    @property
    def bytes_warning(self) -> int:
        ...

    @property
    def quiet(self) -> int:
        ...

    @property
    def hash_randomization(self) -> int:
        ...

    @property
    def isolated(self) -> int:
        ...

    @property
    def dev_mode(self) -> bool:
        ...

    @property
    def utf8_mode(self) -> int:
        ...

    @property
    def warn_default_encoding(self) -> int:
        ...

    @property
    def safe_path(self) -> bool:
        ...

    @property
    def int_max_str_digits(self) -> int:
        ...
float_info: _float_info

@final
@type_check_only
class _float_info(structseq[float], tuple[float, int, int, float, int, int, int, int, float, int, int]):
    __match_args__: Final = ('max', 'max_exp', 'max_10_exp', 'min', 'min_exp', 'min_10_exp', 'dig', 'mant_dig', 'epsilon', 'radix', 'rounds')

    @property
    def max(self) -> float:
        ...

    @property
    def max_exp(self) -> int:
        ...

    @property
    def max_10_exp(self) -> int:
        ...

    @property
    def min(self) -> float:
        ...

    @property
    def min_exp(self) -> int:
        ...

    @property
    def min_10_exp(self) -> int:
        ...

    @property
    def dig(self) -> int:
        ...

    @property
    def mant_dig(self) -> int:
        ...

    @property
    def epsilon(self) -> float:
        ...

    @property
    def radix(self) -> int:
        ...

    @property
    def rounds(self) -> int:
        ...
hash_info: _hash_info

@final
@type_check_only
class _hash_info(structseq[Any | int], tuple[int, int, int, int, int, str, int, int, int]):
    __match_args__: Final = ('width', 'modulus', 'inf', 'nan', 'imag', 'algorithm', 'hash_bits', 'seed_bits', 'cutoff')

    @property
    def width(self) -> int:
        ...

    @property
    def modulus(self) -> int:
        ...

    @property
    def inf(self) -> int:
        ...

    @property
    def nan(self) -> int:
        ...

    @property
    def imag(self) -> int:
        ...

    @property
    def algorithm(self) -> str:
        ...

    @property
    def hash_bits(self) -> int:
        ...

    @property
    def seed_bits(self) -> int:
        ...

    @property
    def cutoff(self) -> int:
        ...
implementation: _implementation

@type_check_only
class _implementation:
    name: str
    version: _version_info
    hexversion: int
    cache_tag: str
int_info: _int_info

@final
@type_check_only
class _int_info(structseq[int], tuple[int, int, int, int]):
    __match_args__: Final = ('bits_per_digit', 'sizeof_digit', 'default_max_str_digits', 'str_digits_check_threshold')

    @property
    def bits_per_digit(self) -> int:
        ...

    @property
    def sizeof_digit(self) -> int:
        ...

    @property
    def default_max_str_digits(self) -> int:
        ...

    @property
    def str_digits_check_threshold(self) -> int:
        ...

@final
@type_check_only
class _thread_info(_UninstantiableStructseq, tuple[_ThreadInfoName, _ThreadInfoLock, str | None]):
    __match_args__: Final = ('name', 'lock', 'version')

    @property
    def name(self) -> Literal['nt', 'pthread', 'pthread-stubs', 'solaris']:
        ...

    @property
    def lock(self) -> Literal['semaphore', 'mutex+cond'] | None:
        ...

    @property
    def version(self) -> str | None:
        ...
thread_info: _thread_info

@final
@type_check_only
class _version_info(_UninstantiableStructseq, tuple[int, int, int, _ReleaseLevel, int]):
    __match_args__: Final = ('major', 'minor', 'micro', 'releaselevel', 'serial')

    @property
    def major(self) -> int:
        ...

    @property
    def minor(self) -> int:
        ...

    @property
    def micro(self) -> int:
        ...

    @property
    def releaselevel(self) -> Literal['alpha', 'beta', 'candidate', 'final']:
        ...

    @property
    def serial(self) -> int:
        ...
version_info: _version_info

def call_tracing(func: Callable[..., Any], args: Any, /) -> Any:
    ...

def _clear_type_cache() -> None:
    ...

def _current_frames() -> dict[int, FrameType]:
    ...

def _getframe(depth: int=0, /) -> FrameType:
    ...

def _current_exceptions() -> dict[int, BaseException | None]:
    ...

def _getframemodulename(depth: int=0) -> str | None:
    ...

def _debugmallocstats() -> None:
    ...

def __displayhook__(object: object, /) -> None:
    ...

def __excepthook__(exctype: type[BaseException], value: BaseException, traceback: TracebackType | None, /) -> None:
    ...

def exc_info() -> OptExcInfo:
    ...

def exception() -> BaseException | None:
    ...

def exit(status: str | int | None=None, /) -> NoReturn:
    ...

def getallocatedblocks() -> int:
    ...

def getdefaultencoding() -> Literal['utf-8']:
    ...

def getfilesystemencoding() -> LiteralString:
    ...

def getfilesystemencodeerrors() -> LiteralString:
    ...

def getrefcount(object: Any, /) -> int:
    ...

def getrecursionlimit() -> int:
    ...

def getsizeof(obj: object, default: int=...) -> int:
    ...

def getswitchinterval() -> float:
    ...

def getprofile() -> ProfileFunction | None:
    ...

def setprofile(function: ProfileFunction | None, /) -> None:
    ...

def gettrace() -> TraceFunction | None:
    ...

def settrace(function: TraceFunction | None, /) -> None:
    ...

@final
@type_check_only
class _WinVersion(_UninstantiableStructseq, tuple[int, int, int, int, str]):

    @property
    def major(self) -> int:
        ...

    @property
    def minor(self) -> int:
        ...

    @property
    def build(self) -> int:
        ...

    @property
    def platform(self) -> int:
        ...

    @property
    def service_pack(self) -> str:
        ...

    @property
    def service_pack_minor(self) -> int:
        ...

    @property
    def service_pack_major(self) -> int:
        ...

    @property
    def suite_mask(self) -> int:
        ...

    @property
    def product_type(self) -> int:
        ...

    @property
    def platform_version(self) -> tuple[int, int, int]:
        ...

def getwindowsversion() -> _WinVersion:
    ...

@overload
def intern(string: LiteralString, /) -> LiteralString:
    ...

@overload
def intern(string: str, /) -> str:
    ...
__interactivehook__: Callable[[], object]

def is_finalizing() -> bool:
    ...

def breakpointhook(*args: Any, **kwargs: Any) -> Any:
    ...

def setrecursionlimit(limit: int, /) -> None:
    ...

def setswitchinterval(interval: float, /) -> None:
    ...

def gettotalrefcount() -> int:
    ...

@type_check_only
class UnraisableHookArgs(Protocol):
    exc_type: type[BaseException]
    exc_value: BaseException | None
    exc_traceback: TracebackType | None
    err_msg: str | None
    object: _object
unraisablehook: Callable[[UnraisableHookArgs], Any]

def __unraisablehook__(unraisable: UnraisableHookArgs, /) -> Any:
    ...

def addaudithook(hook: Callable[[str, tuple[Any, ...]], Any]) -> None:
    ...

def audit(event: str, /, *args: Any) -> None:
    ...

@final
@type_check_only
class _asyncgen_hooks(structseq[_AsyncgenHook], tuple[_AsyncgenHook, _AsyncgenHook]):
    __match_args__: Final = ('firstiter', 'finalizer')

    @property
    def firstiter(self) -> Callable[[AsyncGenerator[Any, Any]], None] | None:
        ...

    @property
    def finalizer(self) -> Callable[[AsyncGenerator[Any, Any]], None] | None:
        ...

def get_asyncgen_hooks() -> _asyncgen_hooks:
    ...

def set_asyncgen_hooks(firstiter: Callable[[AsyncGenerator[Any, Any]], None] | None=..., finalizer: Callable[[AsyncGenerator[Any, Any]], None] | None=...) -> None:
    ...

def _enablelegacywindowsfsencoding() -> None:
    ...

def get_coroutine_origin_tracking_depth() -> int:
    ...

def set_coroutine_origin_tracking_depth(depth: int) -> None:
    ...

def set_int_max_str_digits(maxdigits: int) -> None:
    ...

def get_int_max_str_digits() -> int:
    ...

def getunicodeinternedsize() -> int:
    ...

def deactivate_stack_trampoline() -> None:
    ...

def is_stack_trampoline_active() -> bool:
    ...

def activate_stack_trampoline(backend: str, /) -> NoReturn:
    ...
from . import _monitoring
monitoring = _monitoring
