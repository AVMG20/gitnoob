export default defineNuxtConfig({
  // Tauri serves the built files from disk, so there is no server to render on.
  ssr: false,
  devtools: { enabled: false },
  css: ['~/assets/css/main.css'],
  app: {
    head: {
      title: 'gitnoob',
      meta: [{ name: 'viewport', content: 'width=device-width, initial-scale=1' }]
    }
  },
  vite: {
    // Let the Tauri CLI own the terminal output.
    clearScreen: false,
    // Tauri points at a fixed port, so failing is better than silently moving.
    server: { strictPort: true },
    build: {
      // One stylesheet linked from the page, rather than per-route chunks
      // fetched at runtime: under the `tauri://` protocol a chunk that fails to
      // load takes the layout with it and says nothing about why.
      cssCodeSplit: false
    }
  },
  hooks: {
    'nitro:init'(nitro) {
      nitro.hooks.hook('prerender:generate', (route) => {
        if (!route.fileName?.endsWith('.html') || typeof route.contents !== 'string') return
        // Nuxt marks its stylesheet and module scripts `crossorigin`, which makes
        // them CORS requests. Tauri serves the bundle from the `tauri://`
        // custom protocol, where those requests fail — leaving a blank window
        // with no CSS and no app. Over http in dev they are harmless either way.
        route.contents = route.contents.replace(/\s+crossorigin(="[^"]*")?/g, '')
      })
    }
  }
})
