// Bundles the server (with @aetherhub/shared inlined) into dist/.
import { build } from 'esbuild';
const common = {
  bundle: true,
  platform: 'node',
  target: 'node22',
  format: 'esm',
  packages: 'external',
  sourcemap: true,
  alias: { '@aetherhub/shared': '../shared/src/index.ts', '@aetherhub/rules': '../rules/src/index.ts' },
  banner: { js: "import { createRequire as __cr } from 'node:module'; const require = __cr(import.meta.url);" },
};
await build({ ...common, entryPoints: ['src/index.ts'], outfile: 'dist/index.js' });
await build({ ...common, entryPoints: ['src/cli.ts'], outfile: 'dist/cli.js' });
console.log('server built');
