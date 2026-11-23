import fs from 'node:fs';
import type { FastifyInstance, FastifyReply, FastifyRequest } from 'fastify';
import { z } from 'zod';
import type { CardDef, ExportFormat, Section } from '@aetherhub/shared';
import { exportDeck, FORMAT_BY_ID, matchQuery, MAX_CARD_TAGS, MAX_DECK_TAGS, MAX_TAG_LEN, parseDeckText, parseSearch, SECTIONS } from '@aetherhub/shared';
import { assetPath, fetchImageUrl, storeImage, type AssetKind } from '../assets';
import { requireUser } from '../auth';
import { bad, forbidden, notFound, type Ctx } from '../context';
import { all, get, run } from '../db/sqlite';
import { customDef, type CustomRow } from '../decks';
import { proxyPdf } from '../pdf';

const SectionZ = z.enum(SECTIONS);
const TagZ = z.string().max(MAX_TAG_LEN + 2);
const TagsZ = z.array(TagZ).max(MAX_CARD_TAGS);
const ScopeZ = z.enum(['global', 'deck']);
const idParam = (req: FastifyRequest) => {
  const n = Number((req.params as { id: string }).id);
  if (!Number.isInteger(n)) bad('Bad id');
  return n;
};

export function contentRoutes(r: FastifyInstance, ctx: Ctx) {
  const auth = { preHandler: requireUser };

  // ---------------------------------------------------------------- images
  const sendFile = (reply: FastifyReply, file: string, type: string) =>
    reply.header('Content-Type', type).header('Cache-Control', 'public, max-age=31536000, immutable').send(fs.createReadStream(file));

  r.get('/img/p/:id/:face/:size', async (req, reply) => {
    const { id, face, size } = req.params as { id: string; face: string; size: string };
    const f = await ctx.images.get(id, parseInt(face, 10) || 0, size === 'large' ? 'large' : 'normal');
    if (!f) return reply.code(404).send();
    return sendFile(reply, f, 'image/jpeg');
  });

  r.get('/img/a/:id', async (req, reply) => {
    const f = assetPath(ctx.app, parseInt((req.params as { id: string }).id, 10));
    if (!f) return reply.code(404).send();
    return sendFile(reply, f, 'image/webp');
  });

  r.post('/api/assets', auth, async (req) => {
    const file = await req.file();
    if (!file) bad('No file');
    const kind = (((file.fields.kind as { value?: string } | undefined)?.value || 'card') as AssetKind) || 'card';
    if (!['playmat', 'card', 'override', 'sleeve'].includes(kind)) bad('Bad kind');
    const buf = await file.toBuffer();
    try {
      return { id: await storeImage(ctx.app, buf, kind, req.user!.id) };
    } catch (e) {
      bad((e as Error).message);
    }
  });

  r.post('/api/assets/url', auth, async (req) => {
    const b = z.object({ url: z.string().url(), kind: z.enum(['playmat', 'card', 'override', 'sleeve']) }).parse(req.body);
    try {
      const buf = await fetchImageUrl(b.url);
      return { id: await storeImage(ctx.app, buf, b.kind, req.user!.id) };
    } catch (e) {
      bad((e as Error).message);
    }
  });

  // ---------------------------------------------------------------- cards
  r.get('/api/cards/search', auth, async (req) => {
    const q = z.object({ q: z.string().max(300).default(''), page: z.coerce.number().int().min(0).default(0), tokens: z.string().optional(), limit: z.coerce.number().int().min(1).max(100).default(60) }).parse(req.query);
    const tokens = q.tokens === '1';
    const res = ctx.cards.search(q.q, { limit: q.limit, offset: q.page * q.limit, tokens });
    // Custom cards are merged in on the first page.
    if (q.page === 0 && q.q.trim()) {
      const parsed = parseSearch(q.q);
      const customs = all<CustomRow>(ctx.app.db, 'SELECT * FROM custom_cards ORDER BY name')
        .map(customDef)
        .filter((d) => (tokens ? d.isToken : true) && matchQuery(d, parsed));
      res.cards = [...customs.slice(0, 20), ...res.cards];
      res.total += customs.length;
    }
    return res;
  });

  r.get('/api/cards/autocomplete', auth, async (req) => {
    const q = String((req.query as { q?: string }).q || '');
    return ctx.cards.autocomplete(q);
  });

  // ---------------------------------------------------------------- rulings
  r.get('/api/cards/rulings/:oracleId', auth, async (req) => ({ rulings: ctx.rulings.forOracle((req.params as { oracleId: string }).oracleId), ready: ctx.rulings.ready }));

  /** Searches rulings by card name and by ruling text. Results are grouped per card. */
  r.get('/api/rulings/search', auth, async (req) => {
    const q = z.object({ q: z.string().max(200).default('') }).parse(req.query).q.trim();
    if (!ctx.rulings.ready) return { ready: false, results: [] };
    if (q.length < 2) return { ready: true, results: [] };
    const byName = ctx.cards.oraclesByName(q, 60);
    const counts = ctx.rulings.counts(byName.map((o) => o.oracleId));
    const named = byName.filter((o) => counts[o.oracleId]).slice(0, 12);
    const text = ctx.rulings.searchText(q, 40).filter((t) => !named.some((n) => n.oracleId === t.oracleId));
    const info = ctx.cards.oracleInfo(text.map((t) => t.oracleId));
    const results = [
      ...named.map((o) => ({ oracleId: o.oracleId, name: o.name, key: 'p:' + o.defaultId, match: 'name' as const, rulings: ctx.rulings.forOracle(o.oracleId) })),
      ...text.filter((t) => info[t.oracleId]).map((t) => ({ oracleId: t.oracleId, name: info[t.oracleId].name, key: 'p:' + info[t.oracleId].defaultId, match: 'text' as const, rulings: t.rulings })),
    ];
    const defs = ctx.decks.resolveDefs(results.map((r) => r.key));
    return { ready: true, results: results.map((r) => ({ ...r, def: defs[r.key] ?? null })) };
  });

  r.get('/api/cards/printings/:oracleId', auth, async (req) => ctx.cards.printings((req.params as { oracleId: string }).oracleId));

  r.post('/api/cards/defs', auth, async (req) => {
    const b = z.object({ keys: z.array(z.string().max(80)).max(500) }).parse(req.body);
    return ctx.decks.resolveDefs(b.keys);
  });

  // ---------------------------------------------------------------- custom cards
  const CustomZ = z.object({
    name: z.string().trim().min(1).max(80),
    manaCost: z.string().max(60).default(''),
    typeLine: z.string().max(120).default(''),
    power: z.string().max(6).nullable().optional(),
    toughness: z.string().max(6).nullable().optional(),
    loyalty: z.string().max(6).nullable().optional(),
    oracleText: z.string().max(2000).default(''),
    colors: z.string().regex(/^[WUBRG]*$/).default(''),
    rarity: z.enum(['common', 'uncommon', 'rare', 'mythic']).default('rare'),
    imageAsset: z.number().int().nullable().optional(),
  });

  r.get('/api/custom-cards', auth, async () => {
    return all<CustomRow & { creator: string | null; decks: number }>(
      ctx.app.db,
      `SELECT c.*, u.username AS creator, (SELECT COUNT(DISTINCT deck_id) FROM deck_cards d WHERE d.card_key = 'c:' || c.id) AS decks
       FROM custom_cards c LEFT JOIN users u ON u.id = c.creator_id ORDER BY c.name`,
    ).map((row) => ({ def: customDef(row), creatorId: row.creator_id, creator: row.creator, decks: row.decks }));
  });

  r.post('/api/custom-cards', auth, async (req) => {
    const b = CustomZ.parse(req.body);
    const now = Date.now();
    const res = run(
      ctx.app.db,
      'INSERT INTO custom_cards (creator_id, name, mana_cost, type_line, power, toughness, loyalty, oracle_text, colors, rarity, image_asset, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
      req.user!.id,
      b.name,
      b.manaCost,
      b.typeLine,
      b.power ?? null,
      b.toughness ?? null,
      b.loyalty ?? null,
      b.oracleText,
      b.colors,
      b.rarity,
      b.imageAsset ?? null,
      now,
      now,
    );
    return customDef(get<CustomRow>(ctx.app.db, 'SELECT * FROM custom_cards WHERE id = ?', Number(res.lastInsertRowid))!);
  });

  const canEditCustom = (req: FastifyRequest, id: number) => {
    const row = get<CustomRow>(ctx.app.db, 'SELECT * FROM custom_cards WHERE id = ?', id);
    if (!row) notFound();
    if (row.creator_id !== req.user!.id && !req.user!.isAdmin) forbidden('Only the creator or an admin can change this card');
    return row;
  };

  r.put('/api/custom-cards/:id', auth, async (req) => {
    const id = idParam(req);
    canEditCustom(req, id);
    const b = CustomZ.parse(req.body);
    run(
      ctx.app.db,
      'UPDATE custom_cards SET name = ?, mana_cost = ?, type_line = ?, power = ?, toughness = ?, loyalty = ?, oracle_text = ?, colors = ?, rarity = ?, image_asset = ?, updated_at = ? WHERE id = ?',
      b.name,
      b.manaCost,
      b.typeLine,
      b.power ?? null,
      b.toughness ?? null,
      b.loyalty ?? null,
      b.oracleText,
      b.colors,
      b.rarity,
      b.imageAsset ?? null,
      Date.now(),
      id,
    );
    return customDef(get<CustomRow>(ctx.app.db, 'SELECT * FROM custom_cards WHERE id = ?', id)!);
  });

  r.delete('/api/custom-cards/:id', auth, async (req) => {
    const id = idParam(req);
    canEditCustom(req, id);
    run(ctx.app.db, 'DELETE FROM custom_cards WHERE id = ?', id);
    return { ok: true };
  });

  // ---------------------------------------------------------------- folders
  r.get('/api/folders', auth, async (req) => all(ctx.app.db, 'SELECT id, name, sort FROM folders WHERE user_id = ? ORDER BY sort, name', req.user!.id));
  r.post('/api/folders', auth, async (req) => {
    const b = z.object({ name: z.string().trim().min(1).max(60) }).parse(req.body);
    const res = run(ctx.app.db, 'INSERT INTO folders (user_id, name) VALUES (?, ?)', req.user!.id, b.name);
    return { id: Number(res.lastInsertRowid), name: b.name };
  });
  r.put('/api/folders/:id', auth, async (req) => {
    const b = z.object({ name: z.string().trim().min(1).max(60) }).parse(req.body);
    run(ctx.app.db, 'UPDATE folders SET name = ? WHERE id = ? AND user_id = ?', b.name, idParam(req), req.user!.id);
    return { ok: true };
  });
  r.delete('/api/folders/:id', auth, async (req) => {
    run(ctx.app.db, 'DELETE FROM folders WHERE id = ? AND user_id = ?', idParam(req), req.user!.id);
    return { ok: true };
  });

  // ---------------------------------------------------------------- decks
  const deckOrThrow = (req: FastifyRequest) => {
    const d = ctx.decks.getDeck(idParam(req), req.user!.id);
    if (!d) notFound('Deck not found');
    return d;
  };
  const editable = (req: FastifyRequest) => {
    const id = idParam(req);
    if (!ctx.decks.canEdit(id, req.user!)) forbidden('Only the owner or an admin can edit this deck');
    return id;
  };
  const touched = (id: number) => {
    ctx.decks.recompute(id);
  };

  r.get('/api/decks', auth, async (req) => ctx.decks.listDecks(req.user!.id));

  r.post('/api/decks', auth, async (req) => {
    const b = z.object({ name: z.string().trim().min(1).max(80).default('New deck'), format: z.string().default('commander'), folderId: z.number().int().nullable().optional() }).parse(req.body ?? {});
    if (!FORMAT_BY_ID[b.format]) bad('Unknown format');
    const id = ctx.decks.create(req.user!.id, b.name, b.format, b.folderId ?? null);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.get('/api/decks/:id', auth, async (req) => deckOrThrow(req));

  r.put('/api/decks/:id', auth, async (req) => {
    const id = editable(req);
    const b = z
      .object({
        name: z.string().trim().min(1).max(80).optional(),
        format: z.string().optional(),
        notes: z.string().max(5000).optional(),
        folderId: z.number().int().nullable().optional(),
        coverKey: z.string().max(80).nullable().optional(),
        sleeveAsset: z.number().int().nullable().optional(),
      })
      .parse(req.body);
    if (b.format && !FORMAT_BY_ID[b.format]) bad('Unknown format');
    ctx.decks.update(id, b);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.delete('/api/decks/:id', auth, async (req) => {
    ctx.decks.remove(editable(req));
    return { ok: true };
  });

  r.post('/api/decks/:id/cards', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ key: z.string().max(80), section: SectionZ.default('main'), qty: z.number().int().min(1).max(999).default(1) }).parse(req.body);
    if (!ctx.decks.resolveDefs([b.key])[b.key]) bad('Unknown card');
    ctx.decks.addCard(id, b.key, b.section as Section, b.qty);
    touched(id);
    const deck = ctx.decks.getDeck(id, req.user!.id)!;
    ctx.images.warm(deck.defs[b.key]?.img ?? []);
    return deck;
  });

  r.post('/api/decks/:id/cards/bulk', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ cards: z.array(z.object({ key: z.string().max(80), section: SectionZ, qty: z.number().int().min(1).max(999) })).max(300) }).parse(req.body);
    const defs = ctx.decks.resolveDefs(b.cards.map((c) => c.key));
    for (const c of b.cards) if (defs[c.key]) ctx.decks.addCard(id, c.key, c.section as Section, c.qty);
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.patch('/api/decks/:id/cards/:cardId', auth, async (req) => {
    const id = editable(req);
    const cardId = Number((req.params as { cardId: string }).cardId);
    const b = z
      .object({
        qty: z.number().int().min(0).max(999).optional(),
        section: SectionZ.optional(),
        foil: z.boolean().optional(),
        tags: TagsZ.optional(),
        key: z.string().max(80).optional(),
        frontAsset: z.number().int().nullable().optional(),
        backAsset: z.number().int().nullable().optional(),
      })
      .parse(req.body);
    if (b.key && !ctx.decks.resolveDefs([b.key])[b.key]) bad('Unknown printing');
    try {
      const { tags, ...rest } = b;
      if (tags) ctx.decks.setCardTags(id, req.user!.id, cardId, tags);
      ctx.decks.updateCard(id, cardId, rest as never);
    } catch (e) {
      bad((e as Error).message);
    }
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/cards/tags', auth, async (req) => {
    const id = editable(req);
    const b = z
      .object({
        cardIds: z.array(z.number().int()).max(1000),
        add: z.array(z.union([TagZ, z.object({ name: TagZ, scope: ScopeZ.optional() })])).max(MAX_CARD_TAGS).default([]),
        remove: TagsZ.default([]),
      })
      .parse(req.body);
    ctx.decks.tagCards(id, req.user!.id, b.cardIds, b.add.map((a) => (typeof a === 'string' ? { name: a } : a)), b.remove);
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.put('/api/decks/:id/tags', auth, async (req) => {
    const id = editable(req);
    const hex = z.string().regex(/^#[0-9a-f]{6}$/i);
    const b = z.object({ tags: z.array(z.object({ name: TagZ, color: hex.nullable(), scope: ScopeZ.default('deck') })).max(MAX_DECK_TAGS * 4) }).parse(req.body);
    ctx.decks.setTags(id, b.tags);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/tags/rename', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ from: TagZ, to: TagZ }).parse(req.body);
    ctx.decks.renameTag(id, b.from, b.to);
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/tags/scope', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ name: TagZ, scope: ScopeZ }).parse(req.body);
    ctx.decks.setScope(id, b.name, b.scope);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/tags/delete', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ name: TagZ }).parse(req.body);
    ctx.decks.deleteTag(id, b.name);
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.delete('/api/decks/:id/cards/:cardId', auth, async (req) => {
    const id = editable(req);
    ctx.decks.removeCard(id, Number((req.params as { cardId: string }).cardId));
    touched(id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/clone', auth, async (req) => {
    const src = idParam(req);
    if (!get(ctx.app.db, 'SELECT 1 FROM decks WHERE id = ?', src)) notFound();
    const id = ctx.decks.clone(src, req.user!.id);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.post('/api/decks/:id/pin', auth, async (req) => {
    const b = z.object({ pinned: z.boolean() }).parse(req.body);
    ctx.decks.setPinned(req.user!.id, idParam(req), b.pinned);
    return { ok: true };
  });

  r.post('/api/decks/:id/override-all', auth, async (req) => {
    const id = editable(req);
    const b = z.object({ oracleId: z.string(), front: z.number().int().nullable(), back: z.number().int().nullable() }).parse(req.body);
    const owner = get<{ owner_id: number }>(ctx.app.db, 'SELECT owner_id FROM decks WHERE id = ?', id)!.owner_id;
    ctx.decks.setUserOverride(owner, b.oracleId, b.front, b.back);
    return ctx.decks.getDeck(id, req.user!.id);
  });

  r.get('/api/decks/:id/export', auth, async (req, reply) => {
    const d = deckOrThrow(req);
    const q = req.query as { fmt?: string; tags?: string };
    const globalTags = new Set(d.tags.filter((t) => t.scope === 'global').map((t) => t.name));
    const text = exportDeck(d.cards, d.defs, (q.fmt || 'moxfield') as ExportFormat, { tags: q.tags === '1', globalTags });
    return reply.header('Content-Type', 'text/plain; charset=utf-8').send(text);
  });

  r.post('/api/decks/:id/proxy', auth, async (req, reply) => {
    const id = idParam(req);
    const b = z.object({ paper: z.enum(['A4', 'Letter']).default('A4'), cutLines: z.boolean().default(true), bleed: z.boolean().default(false), cardIds: z.array(z.number().int()).optional() }).parse(req.body ?? {});
    const deck = ctx.decks.getDeck(id, req.user!.id);
    if (!deck) notFound();
    const game = ctx.decks.forGame(id)!;
    const list: { def: CardDef; qty: number }[] = [];
    const chosen = b.cardIds ? deck.cards.filter((c) => b.cardIds!.includes(c.id)) : deck.cards.filter((c) => ['commander', 'companion', 'main'].includes(c.section));
    for (const c of chosen) {
      const variant = c.frontAsset || c.backAsset ? `${c.def}|o${c.frontAsset || 0}.${c.backAsset || 0}` : c.def;
      const def = game.defs[variant] || deck.defs[c.def];
      if (def) list.push({ def, qty: c.qty });
    }
    if (!list.length) bad('No cards to print');
    const pdf = await proxyPdf(ctx.app, ctx.images, list, { paper: b.paper, cutLines: b.cutLines, bleed: b.bleed });
    const name = deck.name.replace(/[^\w -]+/g, '').trim() || 'deck';
    return reply.header('Content-Type', 'application/pdf').header('Content-Disposition', `attachment; filename="${name} proxies.pdf"`).send(Buffer.from(pdf));
  });

  // ---------------------------------------------------------------- import
  r.post('/api/import/parse', auth, async (req) => {
    const b = z.object({ text: z.string().max(200_000) }).parse(req.body);
    const p = parseDeckText(b.text);
    const matched: unknown[] = [];
    const review: unknown[] = [];
    const keys: string[] = [];
    for (const l of p.lines) {
      const id = l.wellFormed ? ctx.cards.matchName(l.name, l.set, l.number, l.section === 'tokens') : null;
      if (id) {
        matched.push({ ...l, key: 'p:' + id });
        keys.push('p:' + id);
      } else {
        const sugg = ctx.cards.suggest(l.name, 3).map((name) => ({ name, key: 'p:' + (ctx.cards.matchName(name) || '') }));
        review.push({ ...l, suggestions: sugg });
      }
    }
    const defs = ctx.decks.resolveDefs(keys);
    return { format: p.format, lineCount: p.lineCount, matched: matched.map((m) => ({ ...(m as object), resolvedName: defs[(m as { key: string }).key]?.name, resolvedSet: defs[(m as { key: string }).key]?.set })), review };
  });

  r.post('/api/import/commit', auth, async (req) => {
    const b = z
      .object({
        name: z.string().trim().min(1).max(80),
        format: z.string(),
        deckId: z.number().int().nullable().optional(),
        cards: z.array(z.object({ key: z.string().max(80), section: SectionZ, qty: z.number().int().min(1).max(999), foil: z.boolean().optional(), tags: TagsZ.optional(), globalTags: TagsZ.optional() })).max(1000),
      })
      .parse(req.body);
    if (!FORMAT_BY_ID[b.format]) bad('Unknown format');
    let id = b.deckId ?? null;
    if (id != null) {
      if (!ctx.decks.canEdit(id, req.user!)) forbidden();
      ctx.decks.update(id, { name: b.name, format: b.format });
    } else id = ctx.decks.create(req.user!.id, b.name, b.format);
    const defs = ctx.decks.resolveDefs(b.cards.map((c) => c.key));
    const ok = b.cards.filter((c) => defs[c.key]);
    ctx.decks.replaceCards(
      id,
      ok.map((c) => ({ ...c, section: c.section as Section })),
    );
    ctx.decks.addGlobalTags(id, ok.filter((c) => c.globalTags?.length).map((c) => ({ key: c.key, tags: c.globalTags! })));
    ctx.images.warm(Object.values(defs).flatMap((d) => d.img));
    return ctx.decks.getDeck(id, req.user!.id);
  });
}
