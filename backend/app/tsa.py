"""HTTPS client for an RFC 3161 time-stamping authority.

The caller supplies the URL. Only a public HTTPS host is accepted, and the
connection uses an address that was checked after DNS resolution. Failures do
not include the URL.
"""

import ipaddress
import socket
import ssl
import threading
from urllib.parse import urlsplit

_REJECTED = "Timestamp authority URL was rejected."
_FAILED = "Timestamp authority request failed."
_MAX_BODY = 64 * 1024
_MAX_URL = 512
_HEADER_CAP = 8192
_TIMEOUT = 10
_DNS_TIMEOUT = 2.0


def validate_tsa_url(url: str) -> tuple[str, str, int, str]:
    """Checks the URL and returns ``(host, ip, port, path)``.

    Every resolved address must be a public unicast address. One private
    address rejects the URL.
    """
    host, port, path = _parts(url)
    addresses = _addresses(host, port)
    return host, addresses[0], port, path


def fetch_timestamp(url: str, request_der: bytes) -> bytes:
    """POSTs a TimeStampReq and returns the response body.

    The host is resolved again. The socket connects only to an address that is
    still public. Redirects are not followed.
    """
    if not isinstance(request_der, (bytes, bytearray)) or not request_der:
        raise ValueError(_FAILED)
    if len(request_der) > _MAX_BODY:
        raise ValueError(_FAILED)
    try:
        host, _ip, port, path = validate_tsa_url(url)
        addresses = _addresses(host, port)
        return _post(host, addresses[0], port, path, bytes(request_der))
    except ValueError as exc:
        if str(exc) in (_REJECTED, _FAILED):
            raise
        raise ValueError(_FAILED) from None
    except Exception:
        raise ValueError(_FAILED) from None


def _parts(url: str) -> tuple[str, int, str]:
    if not isinstance(url, str) or not url or len(url) > _MAX_URL:
        raise ValueError(_REJECTED)
    if any(ch in url for ch in "\r\n\x00 "):
        raise ValueError(_REJECTED)
    try:
        parts = urlsplit(url)
    except ValueError:
        raise ValueError(_REJECTED) from None
    if parts.scheme != "https" or parts.username or parts.password or parts.fragment:
        raise ValueError(_REJECTED)
    host = parts.hostname
    if not host:
        raise ValueError(_REJECTED)
    _screen_host(host)
    port = 443 if parts.port is None else parts.port
    if not isinstance(port, int) or port < 1 or port > 65535:
        raise ValueError(_REJECTED)
    path = parts.path or "/"
    if not path.startswith("/") or "\\" in path:
        raise ValueError(_REJECTED)
    if parts.query:
        path = f"{path}?{parts.query}"
    return host, port, path


def _screen_host(host: str) -> None:
    if not host.isascii() or any(ch in host for ch in "\r\n\x00 %@"):
        raise ValueError(_REJECTED)
    lowered = host.lower().rstrip(".")
    if (
        lowered == "localhost"
        or lowered.endswith(".localhost")
        or lowered.endswith(".local")
        or host.endswith(".")
    ):
        raise ValueError(_REJECTED)
    literal = _literal_ip(host)
    if literal is not None:
        if not _public_ip(literal):
            raise ValueError(_REJECTED)
        return
    if not all(ch.isalnum() or ch in ".-" for ch in host) or ".." in host:
        raise ValueError(_REJECTED)
    labels = host.split(".")
    if any(not label or label.startswith("-") or label.endswith("-") for label in labels):
        raise ValueError(_REJECTED)
    if all(label.isdigit() for label in labels):
        raise ValueError(_REJECTED)


def _literal_ip(host: str):
    try:
        return ipaddress.ip_address(host)
    except ValueError:
        return None


def _public_ip(ip) -> bool:
    if (
        ip.is_multicast
        or ip.is_private
        or ip.is_loopback
        or ip.is_link_local
        or ip.is_unspecified
        or ip.is_reserved
        or not ip.is_global
    ):
        return False
    return True


def _addresses(host: str, port: int) -> list[str]:
    infos = _lookup(host, port)
    found: list[str] = []
    for info in infos:
        ip_text = info[4][0]
        if "%" in ip_text:
            ip_text = ip_text.split("%", 1)[0]
        if ip_text.lower().startswith("::ffff:"):
            ip_text = ip_text.split(":", 3)[-1]
        literal = _literal_ip(ip_text)
        if literal is None or not _public_ip(literal):
            raise ValueError(_REJECTED)
        found.append(ip_text)
    if not found:
        raise ValueError(_REJECTED)
    return found


def _lookup(host: str, port: int):
    outcome: list = []

    def work() -> None:
        try:
            outcome.append(socket.getaddrinfo(host, port, type=socket.SOCK_STREAM))
        except Exception as exc:
            outcome.append(exc)

    thread = threading.Thread(target=work, daemon=True)
    thread.start()
    thread.join(_DNS_TIMEOUT)
    if thread.is_alive() or not outcome or not isinstance(outcome[0], list):
        raise ValueError(_REJECTED)
    return outcome[0]


def _post(host: str, ip: str, port: int, path: str, body: bytes) -> bytes:
    raw = socket.create_connection((ip, port), timeout=_TIMEOUT)
    try:
        tls = ssl.create_default_context().wrap_socket(raw, server_hostname=host)
    except Exception:
        raw.close()
        raise
    try:
        tls.settimeout(_TIMEOUT)
        host_header = f"[{host}]" if ":" in host else host
        if port != 443:
            host_header = f"{host_header}:{port}"
        request = (
            f"POST {path} HTTP/1.1\r\n"
            f"Host: {host_header}\r\n"
            "Content-Type: application/timestamp-query\r\n"
            "Accept: application/timestamp-reply\r\n"
            f"Content-Length: {len(body)}\r\n"
            "Connection: close\r\n"
            "\r\n"
        ).encode("ascii")
        tls.sendall(request + body)
        return _read_response(tls)
    finally:
        tls.close()


def _read_response(tls) -> bytes:
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = tls.recv(1024)
        if not chunk:
            raise ValueError(_FAILED)
        data += chunk
        if len(data) > _HEADER_CAP:
            raise ValueError(_FAILED)
    header, rest = data.split(b"\r\n\r\n", 1)
    lines = header.split(b"\r\n")
    status = lines[0].split()
    if len(status) < 2 or status[1] != b"200":
        raise ValueError(_FAILED)
    headers = {}
    for line in lines[1:]:
        if b":" not in line:
            raise ValueError(_FAILED)
        name, value = line.split(b":", 1)
        headers[name.strip().lower()] = value.strip()
    if b"transfer-encoding" in headers or b"content-length" not in headers:
        raise ValueError(_FAILED)
    try:
        length = int(headers[b"content-length"])
    except ValueError:
        raise ValueError(_FAILED) from None
    if length <= 0 or length > _MAX_BODY:
        raise ValueError(_FAILED)
    body = rest
    while len(body) < length:
        chunk = tls.recv(min(4096, length - len(body)))
        if not chunk:
            raise ValueError(_FAILED)
        body += chunk
    if len(body) > _MAX_BODY:
        raise ValueError(_FAILED)
    return body[:length]
