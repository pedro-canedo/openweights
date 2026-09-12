"""Bounded public HTTPS downloads with DNS pinning and redirect revalidation."""
import http.client
import ipaddress
import socket
import ssl
import time
from urllib.parse import quote, urljoin, urlsplit
from urllib.robotparser import RobotFileParser

AGENT = 'OpenWeightsStudio/2.2'
LIMIT = 32 * 1024**2


def public_target(url):
    u = urlsplit(url)
    if u.scheme != 'https' or not u.hostname or u.username or u.password or u.port not in (None, 443):
        raise ValueError('Use uma URL HTTPS pública, sem credenciais e na porta padrão.')
    if len(url) > 2048 or any(ord(c) < 32 for c in url) or u.fragment or u.query:
        raise ValueError('URL deve ter até 2048 caracteres, sem query, fragmentos ou caracteres de controle.')
    addresses = socket.getaddrinfo(u.hostname, 443, type=socket.SOCK_STREAM)
    ips = sorted({entry[4][0] for entry in addresses})
    if not ips or any(not ipaddress.ip_address(ip).is_global for ip in ips):
        raise ValueError('Endereços locais, privados e reservados não são permitidos.')
    return u, ips[0]


def request(url, limit=LIMIT):
    for _ in range(6):
        u, ip = public_target(url)
        # Connect directly to the validated IP; TLS and Host still use the DNS name.
        conn = http.client.HTTPSConnection(u.hostname, timeout=15)
        raw = socket.create_connection((ip, 443), timeout=15)
        try:
            conn.sock = ssl.create_default_context().wrap_socket(raw, server_hostname=u.hostname)
            conn.request('GET', quote(u.path or '/', safe='/%:@-._~!$&()*+,;='),
                         headers={'User-Agent': AGENT, 'Accept-Encoding': 'identity'})
            response = conn.getresponse()
            if response.status in (301, 302, 303, 307, 308):
                url = urljoin(url, response.getheader('Location', ''))
                continue
            if response.status == 404:
                return b'', 'missing', url
            if response.status != 200:
                raise ValueError(f'Servidor respondeu HTTP {response.status}.')
            if response.getheader('Content-Encoding', 'identity') != 'identity':
                raise ValueError('Compressão HTTP não suportada para ingestão limitada.')
            if int(response.getheader('Content-Length', '0')) > limit:
                raise ValueError('Download excede o limite permitido.')
            chunks, size, started = [], 0, time.monotonic()
            while True:
                part = response.read(min(65536, limit + 1 - size))
                if not part:
                    break
                size += len(part)
                if size > limit or time.monotonic() - started > 60:
                    raise ValueError('Download excedeu tamanho ou duração permitidos.')
                chunks.append(part)
            return b''.join(chunks), response.getheader('Content-Type', '').split(';')[0], url
        finally:
            conn.close()
            raw.close()
    raise ValueError('Muitos redirecionamentos.')


def fetch_page(url):
    u, _ = public_target(url)
    robots_url = f'https://{u.netloc}/robots.txt'
    robots, mime, _ = request(robots_url, 512 * 1024)
    if mime != 'missing':
        parser = RobotFileParser(robots_url)
        parser.parse(robots.decode('utf-8', errors='replace').splitlines())
        if not parser.can_fetch(AGENT, url):
            raise ValueError('robots.txt não permite acessar esta página.')
        delay = parser.crawl_delay(AGENT) or 1
        if delay > 10:
            raise ValueError('Crawl-delay exige espera superior ao limite desta importação.')
        time.sleep(delay)
    raw, mime, final = request(url)
    if final != url:
        # A redirected page may have a different robots policy. Require its explicit URL.
        raise ValueError('A página redireciona. Adicione a URL final diretamente.')
    if mime not in ('text/html', 'application/xhtml+xml', 'text/plain'):
        raise ValueError('URL precisa retornar HTML ou texto. Use upload para outros formatos.')
    return raw, 'page.txt' if mime == 'text/plain' else 'page.html'


def repository_url(url, revision):
    u = urlsplit(url)
    if not revision or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-/' for c in revision):
        raise ValueError('Revisão inválida.')
    if u.scheme != 'https' or u.query or u.fragment or u.username or u.password:
        raise ValueError('Informe a URL HTTPS pública do repositório.')
    parts = u.path.strip('/').removesuffix('.git').split('/')
    if any(not p or p in ('.', '..') or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-' for c in p) for p in parts):
        raise ValueError('Caminho de repositório inválido.')
    ref = quote(revision, safe='')
    if u.netloc == 'github.com' and len(parts) == 2:
        return f'https://codeload.github.com/{parts[0]}/{parts[1]}/zip/{ref}'
    if u.netloc == 'gitlab.com' and len(parts) >= 2:
        return f'https://gitlab.com/{"/".join(parts)}/-/archive/{ref}/{parts[-1]}-{ref}.zip'
    raise ValueError('Use a URL de um repositório público github.com ou gitlab.com.')
