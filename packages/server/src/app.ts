import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import cookie from '@fastify/cookie';
import multipart from '@fastify/multipart';
import fastifyStatic from '@fastify/static';
import Fastify, { type FastifyInstance, type FastifyReply } from 'fastify';
import { WebSocketServer } from 'ws';
import { ZodError } from 'zod';
import { ActivityLog, logDbPath } from './activity';
import { registerActivityLog } from './activityRoutes';
import { COOKIE, userFromToken } from './auth';
import { CardDataService } from './carddata';
import { config } from './config';
import { HttpError, type Ctx } from './context';
import { AppDb } from './db/app';
import { CardDb, cardsDbPath } from './db/cards';
import { RulingsDb, rulingsDbPath } from './db/rulings';
import { DeckService } from './decks';
import { PreconService } from './precons';
import { GameHub } from './games/hub';
import { ImageCache } from './images';
import { logStream, setLogSink } from './serverlog';
import { adminRoutes } from './routes/admin';
import { authRoutes } from './routes/auth';
import { contentRoutes } from './routes/content';
import { draftRoutes } from './routes/draft';
import { playRoutes } from './routes/play';
import { rulesRoutes } from './routes/rules';
import { soloRoutes } from './routes/solo';

function findClientDir(): string | null {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const candidates = [config.clientDir, path.resolve(here, '../../client/dist'), path.resolve(here, '../client'), path.resolve(here, 'client'), path.resolve('packages/client/dist')].filter(Boolean);
  for (const c of candidates) if (fs.existsSync(path.join(c, 'index.html'))) return c;
  return null;
}

export async function buildApp(opts: { logger?: boolean } = {}): Promise<{ f: FastifyInstance; ctx: Ctx }> {
  fs.mkdirSync(config.dataDir, { recursive: true });
  const f = Fastify({
    trustProxy: config.trustProxy,
    logger: opts.logger ? { level: process.env.LOG_LEVEL || 'info', stream: logStream } : false,
    bodyLimit: 12 * 1024 * 1024,
    forceCloseConnections: true,
  });
  const activity = new ActivityLog(logDbPath(config.dataDir), !!opts.logger);
  const log = activity.fn();
  setLogSink((e) => activity.add(e));

  const app = new AppDb();
  const cards = new CardDb(cardsDbPath(config.dataDir));
  const images = new ImageCache();
  const decks = new DeckService(app, cards, new PreconService(app, cards, config.preconsFile));
  try {
    decks.refreshWarnings();
  } catch (e) {
    log(`Deck warnings not refreshed: ${(e as Error).message}`);
  }
  const rulings = new RulingsDb(rulingsDbPath(config.dataDir));
  const carddata = new CardDataService(app, cards, rulings, activity.fn({ cat: 'cards' }));
  const hub = new GameHub(app, cards, decks, images, activity.fn({ cat: 'game' }));
  hub.statusExtra = () => {
    const s = cards.stats();
    return { serverName: app.setting('serverName', 'Aetherhub'), cardDataUpdatedAt: s.importedAt, cardCount: s.oracles, importing: carddata.importing, precons: decks.precons.enabled() && decks.precons.available() > 0 };
  };
  const ctx: Ctx = { app, cards, rulings, images, decks, carddata, hub, activity, log };
  // Resolve the preconstructed decks' cards in the background, so the first visit to the tab is quick.
  if (decks.precons.enabled()) decks.precons.list().catch((e) => log('Preconstructed decks: ' + e.message));

  await f.register(cookie);
  await f.register(multipart, { limits: { fileSize: config.uploadLimit, files: 1 } });

  f.decorateRequest('user', null);
  f.addHook('onRequest', async (req) => {
    req.user = userFromToken(app, req.cookies[COOKIE], req.ip);
  });
  registerActivityLog(f, ctx);

  f.setErrorHandler((err, req, reply) => {
    if (err instanceof ZodError) return reply.code(400).send({ error: err.issues.map((i) => i.message).join('; ') });
    if (err instanceof HttpError) return reply.code(err.status).send({ error: err.message });
    const status = (err as { statusCode?: number }).statusCode;
    if (status && status < 500) return reply.code(status).send({ error: (err as Error).message });
    req.log.error({ err, logged: true });
    if (req.activity) {
      req.activity.error = (err as Error).message;
      req.activity.stack = (err as Error).stack;
    } else log(`Server error: ${(err as Error).message}`, { level: 'error', cat: 'server', user: req.user?.username, detail: (err as Error).stack });
    return reply.code(500).send({ error: 'Server error' });
  });

  const prefix = config.basePath.replace(/\/$/, '');
  await f.register(
    async (r) => {
      authRoutes(r, ctx);
      adminRoutes(r, ctx);
      contentRoutes(r, ctx);
      playRoutes(r, ctx);
      soloRoutes(r, ctx);
      rulesRoutes(r, ctx);
      draftRoutes(r, ctx);
      r.get('/api/health', async () => ({ ok: true, version: config.version }));
    },
    { prefix, logLevel: 'warn' },
  );

  // Static client with SPA fallback; index.html gets the base path injected.
  const clientDir = findClientDir();
  if (clientDir) {
    const indexHtml = fs
      .readFileSync(path.join(clientDir, 'index.html'), 'utf8')
      .replace('<head>', `<head><base href="${config.basePath}"><script>window.__AETHER_BASE__=${JSON.stringify(config.basePath)}</script>`);
    await f.register(fastifyStatic, { root: clientDir, prefix: config.basePath, index: false, wildcard: false, logLevel: 'warn' });
    const sendIndex = (_req: unknown, reply: FastifyReply) => reply.header('Content-Type', 'text/html; charset=utf-8').header('Cache-Control', 'no-cache').send(indexHtml);
    f.get(config.basePath, { logLevel: 'warn' }, sendIndex);
    f.setNotFoundHandler((req, reply) => {
      const url = req.url.split('?')[0];
      if (req.method === 'GET' && url.startsWith(config.basePath) && !url.startsWith(config.basePath + 'api/') && !url.startsWith(config.basePath + 'img/')) {
        const rel = url.slice(config.basePath.length);
        const file = path.join(clientDir, rel);
        if (rel && file.startsWith(clientDir) && fs.existsSync(file) && fs.statSync(file).isFile()) return reply.sendFile(rel);
        return sendIndex(req, reply);
      }
      return reply.code(404).send({ error: 'Not found' });
    });
  }

  // WebSocket on the same port.
  const wss = new WebSocketServer({ noServer: true, maxPayload: 1024 * 1024 });
  f.server.on('upgrade', (req, socket, head) => {
    const url = (req.url || '').split('?')[0];
    if (url !== config.basePath + 'ws') {
      socket.destroy();
      return;
    }
    // Refuse cross-site WebSocket connections (the browser always sends Origin).
    const origin = req.headers.origin;
    if (origin) {
      let ok = false;
      try {
        const o = new URL(origin).host;
        const hosts = [req.headers.host, req.headers['x-forwarded-host']].flat().filter(Boolean) as string[];
        ok = hosts.some((h) => h.split(',')[0].trim() === o);
      } catch {
        ok = false;
      }
      if (!ok) {
        socket.write('HTTP/1.1 403 Forbidden\r\n\r\n');
        socket.destroy();
        return;
      }
    }
    const cookies = f.parseCookie(req.headers.cookie || '');
    const user = userFromToken(app, cookies[COOKIE]);
    if (!user) {
      socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
      socket.destroy();
      return;
    }
    wss.handleUpgrade(req, socket, head, (ws) => hub.attach(ws, user));
  });

  f.addHook('onClose', async () => {
    hub.stop();
    carddata.stop();
    wss.close();
    cards.close();
    rulings.close();
    app.db.close();
    activity.close();
  });

  hub.init(config.heartbeatMs);
  return { f, ctx };
}
