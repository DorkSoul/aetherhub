import { buildApp } from './app';
import { config } from './config';

const { f, ctx } = await buildApp({ logger: true });
ctx.carddata.start();
await f.listen({ port: config.port, host: config.host });
f.log.info(`Aetherhub ${config.version} listening on http://${config.host}:${config.port}${config.basePath} · data in ${config.dataDir}`);

let closing = false;
/** Close the app (onClose hooks save running games), then exit; a hung close still exits after 10 s. */
function shutdown(code: number) {
  if (closing) return;
  closing = true;
  setTimeout(() => process.exit(code), 10_000).unref();
  f.close().then(
    () => process.exit(code),
    () => process.exit(code),
  );
}

for (const sig of ['SIGINT', 'SIGTERM'] as const) {
  process.on(sig, () => {
    f.log.info('Shutting down');
    shutdown(0);
  });
}

// A stray rejected promise is logged instead of taking every game down with it (Node's default is to exit).
process.on('unhandledRejection', (err) => {
  f.log.error({ err }, 'Unhandled promise rejection');
});
// After an uncaught exception the process state is unknown: log it, save what can be saved, and exit so the
// container restarts cleanly.
process.on('uncaughtException', (err) => {
  f.log.fatal({ err }, 'Uncaught exception, shutting down');
  shutdown(1);
});
