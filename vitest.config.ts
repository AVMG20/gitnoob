import { fileURLToPath } from 'node:url'
import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

/**
 * The unit tests stay plain Node, exactly as they were; the component tests
 * say so themselves with a `@vitest-environment happy-dom` line at the top of
 * the file, which keeps one config for both rather than two setups to drift.
 *
 * `~` is aliased because the components import their composables the way
 * Nuxt resolves them, and the SFC compiler is what lets a `.vue` file be
 * mounted at all outside Nuxt.
 */
export default defineConfig({
  plugins: [vue()],
  resolve: {
    alias: {
      '~': fileURLToPath(new URL('./app', import.meta.url))
    }
  },
  test: {
    globals: false
  }
})
