import { defineConfig } from "vite";

export default defineConfig({
  root: "web",
  clearScreen: false,
  build: {
    outDir: "../dist",
    emptyOutDir: true,
  },
});
