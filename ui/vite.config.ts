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
})
