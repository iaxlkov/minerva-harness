import { defineConfig } from 'vite';
export default defineConfig({
 root: 'apps/web',
 server: { host: '127.0.0.1', port: 1420, strictPort: true, fs: { allow: ['../..'] }, proxy: { '/api': { target: 'http://127.0.0.1:3080', ws: true, changeOrigin: true } } },
 build: { outDir: '../../dist', emptyOutDir: true },
});
