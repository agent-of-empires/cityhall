import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  server: {
    // Proxy API calls to the Rust backend during development.
    proxy: {
      "/api": "http://127.0.0.1:3000",
    },
  },
});
