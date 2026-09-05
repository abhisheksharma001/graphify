/// <reference types="vitest/config" />
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// `pnpm dev` serves the UI on its own port, so `/api` has to be pointed at the engine.
// A production build has no proxy and needs none: the engine embeds `ui/dist` and
// serves both from one origin.
export default defineConfig({
  plugins: [react()],
  server: {
    proxy: {
      '/api': { target: 'http://127.0.0.1:3737', changeOrigin: false },
    },
  },
  // The tests live beside what they are about, under `src`, so `tsc -b` type-checks them
  // with everything else and a test that stopped compiling cannot pass by being skipped.
  //
  // `jsdom` rather than a real browser: what is being asserted is what the wizard sends
  // and what it writes on the screen, and neither of those is a question about layout.
  // A browser would be a second thing to install in CI for no assertion it could make
  // that this one cannot.
  test: {
    environment: 'jsdom',
    include: ['src/**/*.test.{ts,tsx}'],
    restoreMocks: true,
  },
})
