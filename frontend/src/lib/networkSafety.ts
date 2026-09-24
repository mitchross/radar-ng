/**
 * Plain http is fine to a server on the user's own network, which is what
 * the app's cleartext allowance is for. Over the internet it exposes every
 * request, so Settings warns about it.
 */
export function isCleartextToPublicHost(url: string): boolean {
  const match = /^http:\/\/(\[[^\]]+\]|[^/:?#]+)/i.exec(url.trim());
  if (!match) return false;
  return !isLocalHost(match[1].toLowerCase());
}

function isLocalHost(host: string): boolean {
  if (host === "localhost" || /\.(local|lan|home\.arpa|internal)$/.test(host)) return true;
  if (!host.includes(".") && !host.startsWith("[")) return true; // single-label LAN name
  if (host.startsWith("[")) {
    const v6 = host.slice(1, -1);
    return v6 === "::1" || /^f[cd][0-9a-f]{2}:/.test(v6) || /^fe[89ab][0-9a-f]:/.test(v6);
  }
  const octets = host.split(".").map(Number);
  if (octets.length !== 4 || octets.some((o) => !Number.isInteger(o) || o < 0 || o > 255)) return false;
  const [a, b] = octets;
  return (
    a === 10 ||
    a === 127 ||
    (a === 172 && b >= 16 && b <= 31) ||
    (a === 192 && b === 168) ||
    (a === 169 && b === 254) ||
    (a === 100 && b >= 64 && b <= 127) // CGNAT, e.g. Tailscale
  );
}
