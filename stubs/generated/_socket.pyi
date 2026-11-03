# _socket.pyi — Arrow 同梱の型スタブ（pandas_import_plan.md の 1-6）。
# ⚠ 生成したファイル。手で書き換えない（scripts/gen_builtin_py_stubs.ps1・CPython 3.12.2 / win32）。
# typeshed（https://github.com/python/typeshed・commit 289e5d3568961c8bcd33d01eef5b7ec5e1ad33ad）の stdlib/_socket.pyi を、
# この CPython の版と OS の分岐だけ残して畳んだもの。typeshed は Apache License 2.0（LICENSE-typeshed.txt）。
import sys
from _typeshed import ReadableBuffer, WriteableBuffer
from collections.abc import Iterable
from socket import error as error, gaierror as gaierror, herror as herror, timeout as timeout
from typing import Any, Final, SupportsIndex, TypeAlias, overload
from typing_extensions import CapsuleType, disjoint_base
AF_INET: Final[int]
AF_INET6: Final[int]
AF_UNSPEC: Final[int]
SOCK_STREAM: Final[int]
SOCK_DGRAM: Final[int]
SOCK_RAW: Final[int]
SOCK_RDM: Final[int]
SOCK_SEQPACKET: Final[int]
SO_ACCEPTCONN: Final[int]
SO_BROADCAST: Final[int]
SO_DEBUG: Final[int]
SO_DONTROUTE: Final[int]
SO_ERROR: Final[int]
SO_KEEPALIVE: Final[int]
SO_LINGER: Final[int]
SO_OOBINLINE: Final[int]
SO_RCVBUF: Final[int]
SO_RCVLOWAT: Final[int]
SO_RCVTIMEO: Final[int]
SO_REUSEADDR: Final[int]
SO_SNDBUF: Final[int]
SO_SNDLOWAT: Final[int]
SO_SNDTIMEO: Final[int]
SO_TYPE: Final[int]
SO_USELOOPBACK: Final[int]
SO_EXCLUSIVEADDRUSE: Final[int]
SOMAXCONN: Final[int]
MSG_CTRUNC: Final[int]
MSG_DONTROUTE: Final[int]
MSG_OOB: Final[int]
MSG_PEEK: Final[int]
MSG_TRUNC: Final[int]
MSG_WAITALL: Final[int]
MSG_ERRQUEUE: Final[int]
MSG_BCAST: Final[int]
MSG_MCAST: Final[int]
SOL_IP: Final[int]
SOL_SOCKET: Final[int]
SOL_TCP: Final[int]
SOL_UDP: Final[int]
IPPROTO_ICMP: Final[int]
IPPROTO_IP: Final[int]
IPPROTO_RAW: Final[int]
IPPROTO_TCP: Final[int]
IPPROTO_UDP: Final[int]
IPPROTO_AH: Final[int]
IPPROTO_DSTOPTS: Final[int]
IPPROTO_EGP: Final[int]
IPPROTO_ESP: Final[int]
IPPROTO_FRAGMENT: Final[int]
IPPROTO_HOPOPTS: Final[int]
IPPROTO_ICMPV6: Final[int]
IPPROTO_IDP: Final[int]
IPPROTO_IGMP: Final[int]
IPPROTO_IPV6: Final[int]
IPPROTO_NONE: Final[int]
IPPROTO_PIM: Final[int]
IPPROTO_PUP: Final[int]
IPPROTO_ROUTING: Final[int]
IPPROTO_SCTP: Final[int]
IPPROTO_GGP: Final[int]
IPPROTO_IPV4: Final[int]
IPPROTO_MAX: Final[int]
IPPROTO_ND: Final[int]
IPPROTO_CBT: Final[int]
IPPROTO_ICLFXBM: Final[int]
IPPROTO_IGP: Final[int]
IPPROTO_L2TP: Final[int]
IPPROTO_PGM: Final[int]
IPPROTO_RDP: Final[int]
IPPROTO_ST: Final[int]
IPPORT_RESERVED: Final[int]
IPPORT_USERRESERVED: Final[int]
INADDR_ALLHOSTS_GROUP: Final[int]
INADDR_ANY: Final[int]
INADDR_BROADCAST: Final[int]
INADDR_LOOPBACK: Final[int]
INADDR_MAX_LOCAL_GROUP: Final[int]
INADDR_NONE: Final[int]
INADDR_UNSPEC_GROUP: Final[int]
IP_ADD_MEMBERSHIP: Final[int]
IP_DROP_MEMBERSHIP: Final[int]
IP_HDRINCL: Final[int]
IP_MULTICAST_IF: Final[int]
IP_MULTICAST_LOOP: Final[int]
IP_MULTICAST_TTL: Final[int]
IP_OPTIONS: Final[int]
IP_RECVDSTADDR: Final[int]
IP_RECVTOS: Final[int]
IP_TOS: Final[int]
IP_TTL: Final[int]
IP_ADD_SOURCE_MEMBERSHIP: Final[int]
IP_BLOCK_SOURCE: Final[int]
IP_DROP_SOURCE_MEMBERSHIP: Final[int]
IP_PKTINFO: Final[int]
IP_UNBLOCK_SOURCE: Final[int]
IPV6_CHECKSUM: Final[int]
IPV6_JOIN_GROUP: Final[int]
IPV6_LEAVE_GROUP: Final[int]
IPV6_MULTICAST_HOPS: Final[int]
IPV6_MULTICAST_IF: Final[int]
IPV6_MULTICAST_LOOP: Final[int]
IPV6_RECVTCLASS: Final[int]
IPV6_TCLASS: Final[int]
IPV6_UNICAST_HOPS: Final[int]
IPV6_V6ONLY: Final[int]
IPV6_DONTFRAG: Final[int]
IPV6_HOPLIMIT: Final[int]
IPV6_HOPOPTS: Final[int]
IPV6_PKTINFO: Final[int]
IPV6_RECVRTHDR: Final[int]
IPV6_RTHDR: Final[int]
EAI_AGAIN: Final[int]
EAI_BADFLAGS: Final[int]
EAI_FAIL: Final[int]
EAI_FAMILY: Final[int]
EAI_MEMORY: Final[int]
EAI_NODATA: Final[int]
EAI_NONAME: Final[int]
EAI_SERVICE: Final[int]
EAI_SOCKTYPE: Final[int]
AI_ADDRCONFIG: Final[int]
AI_ALL: Final[int]
AI_CANONNAME: Final[int]
AI_NUMERICHOST: Final[int]
AI_NUMERICSERV: Final[int]
AI_PASSIVE: Final[int]
AI_V4MAPPED: Final[int]
NI_DGRAM: Final[int]
NI_MAXHOST: Final[int]
NI_MAXSERV: Final[int]
NI_NAMEREQD: Final[int]
NI_NOFQDN: Final[int]
NI_NUMERICHOST: Final[int]
NI_NUMERICSERV: Final[int]
TCP_FASTOPEN: Final[int]
TCP_KEEPCNT: Final[int]
TCP_KEEPINTVL: Final[int]
TCP_MAXSEG: Final[int]
TCP_NODELAY: Final[int]
TCP_KEEPIDLE: Final[int]
SIO_RCVALL: Final[int]
SIO_KEEPALIVE_VALS: Final[int]
SIO_LOOPBACK_FAST_PATH: Final[int]
RCVALL_MAX: Final[int]
RCVALL_OFF: Final[int]
RCVALL_ON: Final[int]
RCVALL_SOCKETLEVELONLY: Final[int]
AF_LINK: Final[int]
has_ipv6: bool
BDADDR_ANY: str = '00:00:00:00:00:00'
BDADDR_LOCAL: str = '00:00:00:FF:FF:FF'
AF_HYPERV: Final[int]
HV_PROTOCOL_RAW: Final[int]
HVSOCKET_CONNECT_TIMEOUT: Final[int]
HVSOCKET_CONNECT_TIMEOUT_MAX: Final[int]
HVSOCKET_CONNECTED_SUSPEND: Final[int]
HVSOCKET_ADDRESS_FLAG_PASSTHRU: Final[int]
HV_GUID_ZERO: str = '00000000-0000-0000-0000-000000000000'
HV_GUID_WILDCARD: str = '00000000-0000-0000-0000-000000000000'
HV_GUID_BROADCAST: str = 'FFFFFFFF-FFFF-FFFF-FFFF-FFFFFFFFFFFF'
HV_GUID_CHILDREN: str = '90DB8B89-0D35-4F79-8CE9-49EA0AC8B7CD'
HV_GUID_LOOPBACK: str = 'E0E16197-DD56-4A10-9195-5EE7A155A838'
HV_GUID_PARENT: str = 'A42E7CDA-D03F-480C-9CC2-A4DE20ABB878'
AF_BLUETOOTH: Final[int]
BTPROTO_RFCOMM: Final[int]
SHUT_RD: Final[int]
SHUT_RDWR: Final[int]
SHUT_WR: Final[int]
AF_APPLETALK: Final[int]
AF_DECnet: Final[int]
AF_IPX: Final[int]
AF_SNA: Final[int]
AF_IRDA: Final[int]

@disjoint_base
class socket:

    @property
    def family(self) -> int:
        ...

    @property
    def type(self) -> int:
        ...

    @property
    def proto(self) -> int:
        ...

    @property
    def timeout(self) -> float | None:
        ...

    def __init__(self, family: int=..., type: int=..., proto: int=..., fileno: SupportsIndex | bytes | None=None) -> None:
        ...

    def bind(self, address: tuple[Any, ...] | str | ReadableBuffer, /) -> None:
        ...

    def close(self) -> None:
        ...

    def connect(self, address: tuple[Any, ...] | str | ReadableBuffer, /) -> None:
        ...

    def connect_ex(self, address: tuple[Any, ...] | str | ReadableBuffer, /) -> int:
        ...

    def detach(self) -> int:
        ...

    def fileno(self) -> int:
        ...

    def getpeername(self) -> Any:
        ...

    def getsockname(self) -> Any:
        ...

    def getsockopt(self, *args: Any, **kwargs: Any) -> Any:
        ...

    def getblocking(self) -> bool:
        ...

    def gettimeout(self) -> float | None:
        ...

    def ioctl(self, control: int, option: int | tuple[int, int, int] | bool, /) -> None:
        ...

    def listen(self, backlog: int=..., /) -> None:
        ...

    def recv(self, bufsize: int, flags: int=0, /) -> bytes:
        ...

    def recvfrom(self, bufsize: int, flags: int=0, /) -> tuple[bytes, Any]:
        ...

    def recvfrom_into(self, buffer: WriteableBuffer, nbytes: int=0, flags: int=0) -> tuple[int, Any]:
        ...

    def recv_into(self, buffer: WriteableBuffer, nbytes: int=0, flags: int=0) -> int:
        ...

    def send(self, data: ReadableBuffer, flags: int=0, /) -> int:
        ...

    def sendall(self, data: ReadableBuffer, flags: int=0, /) -> None:
        ...

    def sendto(self, *args: Any, **kwargs: Any) -> int:
        ...

    def setblocking(self, flag: bool, /) -> None:
        ...

    def settimeout(self, value: float | None, /) -> None:
        ...

    def setsockopt(self, *args: Any, **kwargs: Any) -> None:
        ...

    def share(self, process_id: int, /) -> bytes:
        ...

    def shutdown(self, how: int, /) -> None:
        ...
SocketType = socket

def close(fd: SupportsIndex, /) -> None:
    ...

def dup(fd: SupportsIndex, /) -> int:
    ...

def getaddrinfo(host: bytes | str | None, port: bytes | str | int | None, family: int=..., type: int=0, proto: int=0, flags: int=0) -> list[tuple[int, int, int, str, tuple[str, int] | tuple[str, int, int, int] | tuple[int, bytes]]]:
    ...

def gethostbyname(hostname: str, /) -> str:
    ...

def gethostbyname_ex(hostname: str, /) -> tuple[str, list[str], list[str]]:
    ...

def gethostname() -> str:
    ...

def gethostbyaddr(ip_address: str, /) -> tuple[str, list[str], list[str]]:
    ...

def getnameinfo(sockaddr: tuple[str, int] | tuple[str, int, int, int] | tuple[int, bytes], flags: int, /) -> tuple[str, str]:
    ...

def getprotobyname(protocolname: str, /) -> int:
    ...

def getservbyname(servicename: str, protocolname: str=..., /) -> int:
    ...

def getservbyport(port: int, protocolname: str=..., /) -> str:
    ...

def ntohl(x: int, /) -> int:
    ...

def ntohs(x: int, /) -> int:
    ...

def htonl(x: int, /) -> int:
    ...

def htons(x: int, /) -> int:
    ...

def inet_aton(ip_addr: str, /) -> bytes:
    ...

def inet_ntoa(packed_ip: ReadableBuffer, /) -> str:
    ...

def inet_pton(address_family: int, ip_string: str, /) -> bytes:
    ...

def inet_ntop(address_family: int, packed_ip: ReadableBuffer, /) -> str:
    ...

def getdefaulttimeout() -> float | None:
    ...

def setdefaulttimeout(timeout: float | None, /) -> None:
    ...

def if_nameindex() -> list[tuple[int, str]]:
    ...

def if_nametoindex(oname: str, /) -> int:
    ...

def if_indextoname(index: int, /) -> str:
    ...
CAPI: CapsuleType
