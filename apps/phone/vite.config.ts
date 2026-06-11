import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import basicSsl from "@vitejs/plugin-basic-ssl";

export default defineConfig({
  plugins: [react(), basicSsl()],
  server: {
    port: 1421,
    strictPort: true,
    host: "0.0.0.0",
    proxy: {
      "/agenteye/ws": {
        target: "ws://127.0.0.1:17891",
        ws: true,
        rewrite: () => "/ws",
      },
      "/agenteye/video/push": {
        target: "ws://127.0.0.1:17891",
        ws: true,
        rewrite: () => "/video/push",
      },
    },
  },
  build: {
    target: "es2022",
    minify: "esbuild",
  },
});
