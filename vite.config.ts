import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
export default defineConfig({
  optimizeDeps: { include: ["@tauri-apps/api/window"] },
  plugins: [
    react(),
    // Production builds swap the browser preview's sample project for a stub (demo.off.ts).
    {
      name: "no-browser-demo",
      apply: "build",
      enforce: "pre",
      resolveId(source, importer) {
        if (source === "./demo" && importer?.includes("/src/"))
          return fileURLToPath(new URL("./src/demo.off.ts", import.meta.url));
      },
    },
  ],
  test: { include: ["src/**/*.test.{ts,tsx}"] },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: [
        "**/.tools/**",
        "**/target/**",
        "**/build/**",
        "**/src-tauri/gen/**",
        "**/src-tauri/src/**/*.rs",
      ],
    },
  },
  clearScreen: false,
});
