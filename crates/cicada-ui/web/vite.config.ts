// Builds the console for the cicada-ui crate, which embeds it: build.rs runs this with CICADA_UI_OUT_DIR set to a
// folder in Cargo's OUT_DIR; a plain `npm run build` writes to dist/.
// The page is served with a strict Content Security Policy (no inline script or style), so nothing may be inlined.
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

const outDir = process.env.CICADA_UI_OUT_DIR ?? fileURLToPath(new URL('./dist', import.meta.url));

export default defineConfig({
  // Relative links: the console can be mounted under any path.
  base: './',
  build: {
    outDir,
    emptyOutDir: true,
    // No inline polyfill script and no data: URIs.
    modulePreload: { polyfill: false },
    assetsInlineLimit: 0,
  },
  test: {
    include: ['test/**/*.test.ts'],
  },
});
