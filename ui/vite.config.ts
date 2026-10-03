import { defineConfig } from "vitest/config";
import preact from "@preact/preset-vite";

export default defineConfig({
  plugins: [preact()],
  clearScreen: false,
  build: { target: "es2022", outDir: "dist", emptyOutDir: true },
});
