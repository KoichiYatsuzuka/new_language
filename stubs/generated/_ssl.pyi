# _ssl.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_ssl.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer, StrOrBytesPath
from collections.abc import Callable
from ssl import SSLCertVerificationError as SSLCertVerificationError, SSLContext, SSLEOFError as SSLEOFError, SSLError as SSLError, SSLObject, SSLSyscallError as SSLSyscallError, SSLWantReadError as SSLWantReadError, SSLWantWriteError as SSLWantWriteError, SSLZeroReturnError as SSLZeroReturnError
from typing import Any, ClassVar, Final, Literal, TypeAlias, TypedDict, final, overload, type_check_only
from typing_extensions import NotRequired, Self, deprecated, disjoint_base

@type_check_only
class _Cipher(TypedDict):
    aead: bool
    alg_bits: int
    auth: str
    description: str
    digest: str | None
    id: int
    kea: str
    name: str
    protocol: str
    strength_bits: int
    symmetric: str

@type_check_only
class _CertInfo(TypedDict):
    subject: tuple[tuple[tuple[str, str], ...], ...]
    issuer: tuple[tuple[tuple[str, str], ...], ...]
    version: int
    serialNumber: str
    notBefore: str
    notAfter: str
    subjectAltName: NotRequired[tuple[tuple[str, str], ...] | None]
    OCSP: NotRequired[tuple[str, ...] | None]
    caIssuers: NotRequired[tuple[str, ...] | None]
    crlDistributionPoints: NotRequired[tuple[str, ...] | None]

def RAND_add(string: str | ReadableBuffer, entropy: float, /) -> None:
    ...

def RAND_bytes(n: int, /) -> bytes:
    ...

def RAND_status() -> bool:
    ...

def get_default_verify_paths() -> tuple[str, str, str, str]:
    ...

def enum_certificates(store_name: str) -> list[tuple[bytes, str, set[str] | bool]]:
    ...

def enum_crls(store_name: str) -> list[tuple[bytes, str, set[str] | bool]]:
    ...

def txt2obj(txt: str, name: bool=False) -> tuple[int, str, str, str]:
    ...

def nid2obj(nid: int, /) -> tuple[int, str, str, str]:
    ...

@disjoint_base
class _SSLContext:
    check_hostname: bool
    keylog_filename: str | None
    maximum_version: int
    minimum_version: int
    num_tickets: int
    options: int
    post_handshake_auth: bool
    protocol: int
    security_level: int
    sni_callback: Callable[[SSLObject, str, SSLContext], None | int] | None
    verify_flags: int
    verify_mode: int

    def __init__(self, protocol: int, /) -> None:
        ...

    def cert_store_stats(self) -> dict[str, int]:
        ...

    def get_ca_certs(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def get_ciphers(self) -> list[_Cipher]:
        ...

    def load_cert_chain(self, certfile: StrOrBytesPath, keyfile: StrOrBytesPath | None=None, password: Callable[[], str | bytes | bytearray] | str | bytes | bytearray | None=None) -> None:
        ...

    def load_dh_params(self, path: str, /) -> None:
        ...

    def load_verify_locations(self, cafile: StrOrBytesPath | None=None, capath: StrOrBytesPath | None=None, cadata: str | ReadableBuffer | None=None) -> None:
        ...

    def session_stats(self) -> dict[str, int]:
        ...

    def set_ciphers(self, cipherlist: str, /) -> None:
        ...

    def set_default_verify_paths(self) -> None:
        ...

    def set_ecdh_curve(self, name: str, /) -> None:
        ...

@final
class MemoryBIO:
    eof: bool
    pending: int

    def __init__(self) -> None:
        ...

    def read(self, size: int=-1, /) -> bytes:
        ...

    def write(self, b: ReadableBuffer, /) -> int:
        ...

    def write_eof(self) -> None:
        ...

@final
class SSLSession:
    __hash__: ClassVar[None]

    @property
    def has_ticket(self) -> bool:
        ...

    @property
    def id(self) -> bytes:
        ...

    @property
    def ticket_lifetime_hint(self) -> int:
        ...

    @property
    def time(self) -> int:
        ...

    @property
    def timeout(self) -> int:
        ...

@final
class Certificate:

    def get_info(self) -> _CertInfo:
        ...

    def public_bytes(self, *args: Any, **kwargs: Any) -> Any:
        ...
_DEFAULT_CIPHERS: Final[str]
SSL_ERROR_ZERO_RETURN: int = 6
SSL_ERROR_WANT_READ: int = 2
SSL_ERROR_WANT_WRITE: int = 3
SSL_ERROR_WANT_X509_LOOKUP: int = 4
SSL_ERROR_SYSCALL: int = 5
SSL_ERROR_SSL: int = 1
SSL_ERROR_WANT_CONNECT: int = 7
SSL_ERROR_EOF: int = 8
SSL_ERROR_INVALID_ERROR_CODE: int = 10
CERT_NONE: int = 0
CERT_OPTIONAL: int = 1
CERT_REQUIRED: int = 2
VERIFY_DEFAULT: int = 0
VERIFY_CRL_CHECK_LEAF: int = 4
VERIFY_CRL_CHECK_CHAIN: int = 12
VERIFY_X509_STRICT: int = 32
VERIFY_X509_TRUSTED_FIRST: int = 32768
VERIFY_ALLOW_PROXY_CERTS: int = 64
VERIFY_X509_PARTIAL_CHAIN: int = 524288
ALERT_DESCRIPTION_CLOSE_NOTIFY: int = 0
ALERT_DESCRIPTION_UNEXPECTED_MESSAGE: int = 10
ALERT_DESCRIPTION_BAD_RECORD_MAC: int = 20
ALERT_DESCRIPTION_RECORD_OVERFLOW: int = 22
ALERT_DESCRIPTION_DECOMPRESSION_FAILURE: int = 30
ALERT_DESCRIPTION_HANDSHAKE_FAILURE: int = 40
ALERT_DESCRIPTION_BAD_CERTIFICATE: int = 42
ALERT_DESCRIPTION_UNSUPPORTED_CERTIFICATE: int = 43
ALERT_DESCRIPTION_CERTIFICATE_REVOKED: int = 44
ALERT_DESCRIPTION_CERTIFICATE_EXPIRED: int = 45
ALERT_DESCRIPTION_CERTIFICATE_UNKNOWN: int = 46
ALERT_DESCRIPTION_ILLEGAL_PARAMETER: int = 47
ALERT_DESCRIPTION_UNKNOWN_CA: int = 48
ALERT_DESCRIPTION_ACCESS_DENIED: int = 49
ALERT_DESCRIPTION_DECODE_ERROR: int = 50
ALERT_DESCRIPTION_DECRYPT_ERROR: int = 51
ALERT_DESCRIPTION_PROTOCOL_VERSION: int = 70
ALERT_DESCRIPTION_INSUFFICIENT_SECURITY: int = 71
ALERT_DESCRIPTION_INTERNAL_ERROR: int = 80
ALERT_DESCRIPTION_USER_CANCELLED: int = 90
ALERT_DESCRIPTION_NO_RENEGOTIATION: int = 100
ALERT_DESCRIPTION_UNSUPPORTED_EXTENSION: int = 110
ALERT_DESCRIPTION_CERTIFICATE_UNOBTAINABLE: int = 111
ALERT_DESCRIPTION_UNRECOGNIZED_NAME: int = 112
ALERT_DESCRIPTION_BAD_CERTIFICATE_STATUS_RESPONSE: int = 113
ALERT_DESCRIPTION_BAD_CERTIFICATE_HASH_VALUE: int = 114
ALERT_DESCRIPTION_UNKNOWN_PSK_IDENTITY: int = 115
PROTOCOL_SSLv23: int = 2
PROTOCOL_TLS: int = 2
PROTOCOL_TLS_CLIENT: int = 16
PROTOCOL_TLS_SERVER: int = 17
PROTOCOL_TLSv1: int = 3
PROTOCOL_TLSv1_1: int = 4
PROTOCOL_TLSv1_2: int = 5
OP_ALL: Final[int]
OP_NO_SSLv2: int = 0
OP_NO_SSLv3: int = 33554432
OP_NO_TLSv1: int = 67108864
OP_NO_TLSv1_1: int = 268435456
OP_NO_TLSv1_2: int = 134217728
OP_NO_TLSv1_3: int = 536870912
OP_CIPHER_SERVER_PREFERENCE: int = 4194304
OP_SINGLE_DH_USE: int = 0
OP_NO_TICKET: int = 16384
OP_SINGLE_ECDH_USE: int = 0
OP_NO_COMPRESSION: int = 131072
OP_ENABLE_MIDDLEBOX_COMPAT: int = 1048576
OP_NO_RENEGOTIATION: int = 1073741824
OP_IGNORE_UNEXPECTED_EOF: int = 128
OP_LEGACY_SERVER_CONNECT: int = 4
OP_ENABLE_KTLS: int = 8
HOSTFLAG_ALWAYS_CHECK_SUBJECT: int = 1
HOSTFLAG_NEVER_CHECK_SUBJECT: int = 32
HOSTFLAG_NO_WILDCARDS: int = 2
HOSTFLAG_NO_PARTIAL_WILDCARDS: int = 4
HOSTFLAG_MULTI_LABEL_WILDCARDS: int = 8
HOSTFLAG_SINGLE_LABEL_SUBDOMAINS: int = 16
ENCODING_PEM: int = 1
ENCODING_DER: int = 2
PROTO_MINIMUM_SUPPORTED: int = -2
PROTO_MAXIMUM_SUPPORTED: int = -1
PROTO_SSLv3: Final[int]
PROTO_TLSv1: Final[int]
PROTO_TLSv1_1: Final[int]
PROTO_TLSv1_2: Final[int]
PROTO_TLSv1_3: Final[int]
HAS_SNI: Final[bool]
HAS_TLS_UNIQUE: Final[bool]
HAS_ECDH: Final[bool]
HAS_NPN: Final[bool]
HAS_ALPN: Final[bool]
HAS_SSLv2: Final[bool]
HAS_SSLv3: Final[bool]
HAS_TLSv1: Final[bool]
HAS_TLSv1_1: Final[bool]
HAS_TLSv1_2: Final[bool]
HAS_TLSv1_3: Final[bool]
OPENSSL_VERSION_NUMBER: Final[int]
OPENSSL_VERSION_INFO: Final[tuple[int, int, int, int, int]]
OPENSSL_VERSION: Final[str]
_OPENSSL_API_VERSION: Final[tuple[int, int, int, int, int]]
