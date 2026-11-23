import http from 'node:http';
import type { AddressInfo } from 'node:net';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { fetchImageUrl } from './assets';
import { isLanIp, isPrivateTarget } from './lan';

describe('isLanIp / isPrivateTarget', () => {
  it('knows the LAN ranges in every spelling', () => {
    for (const ip of ['10.1.2.3', '127.0.0.1', '172.20.0.1', '192.168.1.5', '169.254.1.1', '100.100.1.1', '::1', 'fd12::1', 'fe80::1', 'fe9f::1', '::ffff:192.168.1.5', '::ffff:c0a8:105', '[::ffff:7f00:1]', 'fe80::1%eth0'])
      expect([ip, isLanIp(ip)]).toEqual([ip, true]);
    for (const ip of ['8.8.8.8', '203.0.113.7', '2606:4700::1111', '::ffff:8.8.8.8', '172.32.0.1', '', 'localhost'])
      expect([ip, isLanIp(ip)]).toEqual([ip, false]);
  });

  it('blocks private and special-purpose targets, including IPv4-mapped IPv6 written in hex', () => {
    for (const ip of ['::ffff:7f00:1', '::ffff:ac11:1', '0.0.0.0', '::', '224.0.0.1', '255.255.255.255', '198.18.0.1', '64:ff9b::7f00:1', '2002:7f00:1::', 'ff02::1', '::7f00:1', 'not-an-ip'])
      expect([ip, isPrivateTarget(ip)]).toEqual([ip, true]);
    for (const ip of ['8.8.8.8', '2606:4700::1111', '151.101.1.1'])
      expect([ip, isPrivateTarget(ip)]).toEqual([ip, false]);
  });
});

describe('fetchImageUrl SSRF guard', () => {
  let server: http.Server;
  let port = 0;
  let hits = 0;
  beforeAll(async () => {
    server = http.createServer((_req, res) => {
      hits++;
      res.writeHead(200, { 'Content-Type': 'image/png' }).end('x');
    });
    await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
    port = (server.address() as AddressInfo).port;
  });
  afterAll(() => new Promise<void>((r) => server.close(() => r())));

  it('refuses loopback by literal, by mapped IPv6 and by a name that resolves to it', async () => {
    for (const url of [`http://127.0.0.1:${port}/a.png`, `http://[::ffff:7f00:1]:${port}/a.png`, `http://[::ffff:127.0.0.1]:${port}/a.png`, `http://localhost:${port}/a.png`])
      await expect(fetchImageUrl(url), url).rejects.toThrow(/not allowed/);
    expect(hits).toBe(0);
  });
});
