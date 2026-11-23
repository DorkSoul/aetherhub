import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

const target = process.env.AETHER_SERVER || 'http://127.0.0.1:8080';

export default defineConfig({
  base: './',
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': target,
      '/img': target,
      '/ws': { target: target.replace(/^http/, 'ws'), ws: true },
    },
  },
  build: { outDir: 'dist', sourcemap: true, chunkSizeWarningLimit: 1500 },
});
