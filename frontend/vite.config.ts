import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { VitePWA } from "vite-plugin-pwa";

/**
 * Captive-portal nginx serves the chess PWA under /app/.
 * CI and Dockerfile pass --base=/app/; this default keeps local
 * `npm run build` aligned with that layout.
 * Dev server (`vite`) still uses "/" unless VITE_BASE is set.
 */
export default defineConfig(({ command }) => {
  const base =
    process.env.VITE_BASE ?? (command === "build" ? "/app/" : "/");

  return {
    base,
    plugins: [
      react(),
      VitePWA({
        registerType: "autoUpdate",
        includeAssets: ["favicon.svg", "icons.svg"],
        manifest: {
          name: "MessyMash · MossyMesh",
          short_name: "MessyMash",
          description:
            "Offline-first decentralized chess on a mesh captive portal. Play locally; sync via island peers.",
          theme_color: "#121212",
          background_color: "#121212",
          display: "standalone",
          orientation: "portrait-primary",
          // Relative to base so /app/ deploys install/scope correctly
          start_url: "./",
          scope: "./",
          lang: "en",
          categories: ["games", "utilities"],
          icons: [
            { src: "favicon.svg", sizes: "any", type: "image/svg+xml", purpose: "any" },
            { src: "favicon.svg", sizes: "any", type: "image/svg+xml", purpose: "maskable" },
          ],
        },
        workbox: {
          globPatterns: ["**/*.{js,css,html,ico,png,svg,webp,woff2,wasm}"],
          // Resolved against Vite `base` by vite-plugin-pwa
          navigateFallback: "index.html",
          navigateFallbackDenylist: [/^\/api\//],
          runtimeCaching: [
            {
              urlPattern: /^https:\/\/fonts\.googleapis\.com\/.*/i,
              handler: "CacheFirst",
              options: {
                cacheName: "google-fonts-stylesheets",
                expiration: { maxEntries: 8, maxAgeSeconds: 60 * 60 * 24 * 365 },
              },
            },
            {
              urlPattern: /^https:\/\/fonts\.gstatic\.com\/.*/i,
              handler: "CacheFirst",
              options: {
                cacheName: "google-fonts-webfonts",
                expiration: { maxEntries: 16, maxAgeSeconds: 60 * 60 * 24 * 365 },
                cacheableResponse: { statuses: [0, 200] },
              },
            },
            {
              urlPattern: /\/api\/.*/i,
              handler: "NetworkFirst",
              options: {
                cacheName: "mesh-api",
                networkTimeoutSeconds: 3,
                expiration: { maxEntries: 32, maxAgeSeconds: 60 * 60 * 24 },
                cacheableResponse: { statuses: [0, 200] },
              },
            },
          ],
        },
        devOptions: { enabled: false },
      }),
    ],
    server: {
      proxy: {
        "/api": {
          target: process.env.MESH_API_PROXY ?? "http://127.0.0.1:8787",
          changeOrigin: true,
        },
      },
    },
    preview: {
      proxy: {
        "/api": {
          target: process.env.MESH_API_PROXY ?? "http://127.0.0.1:8787",
          changeOrigin: true,
        },
      },
    },
  };
});
