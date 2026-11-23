import fs from 'node:fs';
import { PDFDocument, rgb } from 'pdf-lib';
import sharp from 'sharp';
import type { CardDef } from '@aetherhub/shared';
import type { AppDb } from './db/app';
import { assetPath } from './assets';
import type { ImageCache } from './images';

const MM = 72 / 25.4;
const PAPER = { A4: [210, 297], Letter: [215.9, 279.4] } as const;
const CW = 63;
const CH = 88;

function esc(s: string) {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function wrap(text: string, max: number): string[] {
  const out: string[] = [];
  for (const para of text.split('\n')) {
    let line = '';
    for (const w of para.split(' ')) {
      if ((line + ' ' + w).trim().length > max) {
        out.push(line.trim());
        line = w;
      } else line += ' ' + w;
    }
    out.push(line.trim());
  }
  return out;
}

/** A plain text proxy for cards without an image (custom cards, missing scans). */
async function textCard(d: CardDef): Promise<Buffer> {
  const lines = wrap(d.oracleText || '', 38).slice(0, 18);
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="630" height="880">
  <rect width="630" height="880" rx="30" fill="#15130f"/>
  <rect x="24" y="24" width="582" height="832" rx="18" fill="#efe7d2"/>
  <text x="44" y="80" font-family="serif" font-size="34" font-weight="bold">${esc(d.name)}</text>
  <text x="586" y="80" font-family="sans-serif" font-size="26" text-anchor="end">${esc(d.manaCost)}</text>
  <rect x="44" y="104" width="542" height="330" fill="#cfc6ae"/>
  <text x="44" y="480" font-family="sans-serif" font-size="24" font-weight="bold">${esc(d.typeLine)}</text>
  ${lines.map((l, i) => `<text x="44" y="${530 + i * 30}" font-family="sans-serif" font-size="22">${esc(l)}</text>`).join('')}
  ${d.power != null ? `<text x="586" y="830" font-family="serif" font-size="36" font-weight="bold" text-anchor="end">${esc(d.power + '/' + d.toughness)}</text>` : ''}
</svg>`;
  return sharp(Buffer.from(svg)).jpeg({ quality: 90 }).toBuffer();
}

export async function imageForRef(app: AppDb, images: ImageCache, ref: string | undefined): Promise<Buffer | null> {
  if (!ref) return null;
  if (ref.startsWith('a/')) {
    const f = assetPath(app, parseInt(ref.slice(2), 10));
    return f ? sharp(fs.readFileSync(f)).jpeg({ quality: 92 }).toBuffer() : null;
  }
  const m = ref.match(/^p\/([0-9a-f-]{36})\/(\d)$/);
  if (!m) return null;
  const f = await images.get(m[1], +m[2], 'large');
  return f ? fs.readFileSync(f) : null;
}

export async function proxyPdf(
  app: AppDb,
  images: ImageCache,
  cards: { def: CardDef; qty: number }[],
  opts: { paper: 'A4' | 'Letter'; cutLines: boolean; bleed: boolean },
): Promise<Uint8Array> {
  const doc = await PDFDocument.create();
  const [pw, ph] = PAPER[opts.paper];
  const gap = opts.bleed ? 2 : 0;
  const gridW = 3 * CW + 2 * gap;
  const gridH = 3 * CH + 2 * gap;
  const x0 = (pw - gridW) / 2;
  const y0 = (ph - gridH) / 2;
  const list: CardDef[] = [];
  for (const c of cards) for (let i = 0; i < c.qty; i++) list.push(c.def);
  const cache = new Map<string, Awaited<ReturnType<typeof doc.embedJpg>>>();
  for (let p = 0; p < list.length; p += 9) {
    const page = doc.addPage([pw * MM, ph * MM]);
    for (let i = 0; i < 9 && p + i < list.length; i++) {
      const d = list[p + i];
      const col = i % 3;
      const row = Math.floor(i / 3);
      const x = x0 + col * (CW + gap);
      const yTop = y0 + row * (CH + gap);
      const y = ph - yTop - CH;
      const key = d.img[0] || 'text:' + d.key;
      let img = cache.get(key);
      if (!img) {
        const buf = (await imageForRef(app, images, d.img[0])) ?? (await textCard(d));
        img = await doc.embedJpg(buf);
        cache.set(key, img);
      }
      const b = opts.bleed ? 1 : 0;
      page.drawImage(img, { x: (x - b) * MM, y: (y - b) * MM, width: (CW + 2 * b) * MM, height: (CH + 2 * b) * MM });
      if (opts.cutLines) {
        const c = rgb(0.55, 0.55, 0.55);
        const L = 4;
        const corners: [number, number][] = [
          [x, y],
          [x + CW, y],
          [x, y + CH],
          [x + CW, y + CH],
        ];
        for (const [cx, cy] of corners) {
          page.drawLine({ start: { x: (cx - L) * MM, y: cy * MM }, end: { x: (cx + L) * MM, y: cy * MM }, thickness: 0.3, color: c });
          page.drawLine({ start: { x: cx * MM, y: (cy - L) * MM }, end: { x: cx * MM, y: (cy + L) * MM }, thickness: 0.3, color: c });
        }
      }
    }
  }
  return doc.save();
}
