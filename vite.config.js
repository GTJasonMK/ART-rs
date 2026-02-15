import { defineConfig } from "vite";

const port = Number(process.env.VITE_DEV_SERVER_PORT || process.env.PORT || 5173);

export default defineConfig({
  clearScreen: false,
  server: {
    host: "127.0.0.1",
    port,
    strictPort: true
  },
  preview: {
    host: "127.0.0.1",
    port: port + 1,
    strictPort: false
  }
});
