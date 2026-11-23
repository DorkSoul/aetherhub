import net from 'node:net';

// net.BlockList matches IPv4 rules against IPv4-mapped IPv6 addresses in every spelling (::ffff:127.0.0.1 and
// ::ffff:7f00:1 alike), so no string munging is needed.

/** Loopback, RFC 1918, link-local, CGNAT (Tailscale etc.) and IPv6 ULA / link-local. */
const LAN = new net.BlockList();
LAN.addSubnet('10.0.0.0', 8, 'ipv4');
LAN.addSubnet('127.0.0.0', 8, 'ipv4');
LAN.addSubnet('172.16.0.0', 12, 'ipv4');
LAN.addSubnet('192.168.0.0', 16, 'ipv4');
LAN.addSubnet('169.254.0.0', 16, 'ipv4');
LAN.addSubnet('100.64.0.0', 10, 'ipv4');
LAN.addAddress('::1', 'ipv6');
LAN.addSubnet('fc00::', 7, 'ipv6');
LAN.addSubnet('fe80::', 10, 'ipv6');

/** Other special-purpose ranges server-side URL fetches must never reach (checked together with LAN). */
const PRIVATE = new net.BlockList();
PRIVATE.addSubnet('0.0.0.0', 8, 'ipv4'); // "this network"
PRIVATE.addSubnet('192.0.0.0', 24, 'ipv4'); // IETF protocol assignments
PRIVATE.addSubnet('198.18.0.0', 15, 'ipv4'); // benchmarking
PRIVATE.addSubnet('224.0.0.0', 3, 'ipv4'); // multicast, reserved, broadcast
PRIVATE.addSubnet('::', 96, 'ipv6'); // unspecified, loopback, IPv4-compatible
PRIVATE.addSubnet('64:ff9b::', 96, 'ipv6'); // NAT64 (maps to IPv4)
PRIVATE.addSubnet('64:ff9b:1::', 48, 'ipv6'); // local-use NAT64
PRIVATE.addSubnet('2002::', 16, 'ipv6'); // 6to4 (embeds an IPv4 address)
PRIVATE.addSubnet('ff00::', 8, 'ipv6'); // multicast

/** Strips URL brackets and an IPv6 zone id; returns the address and its family, or null when it isn't an IP. */
function parse(ip: string | undefined | null): { a: string; family: 'ipv4' | 'ipv6' } | null {
  if (!ip) return null;
  const a = ip.trim().replace(/^\[|\]$/g, '').replace(/%.*$/, '');
  const v = net.isIP(a);
  return v === 4 ? { a, family: 'ipv4' } : v === 6 ? { a, family: 'ipv6' } : null;
}

/** True for loopback, RFC1918, link-local, CGNAT (Tailscale etc.) and IPv6 ULA/link-local addresses. */
export function isLanIp(ip: string | undefined | null): boolean {
  const p = parse(ip);
  return !!p && LAN.check(p.a, p.family);
}

/** Private/reserved ranges that server-side URL fetches must never reach (SSRF guard). Anything that doesn't parse
 *  as an IP address counts as private too. */
export function isPrivateTarget(ip: string): boolean {
  const p = parse(ip);
  return !p || LAN.check(p.a, p.family) || PRIVATE.check(p.a, p.family);
}
