import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// Tauri expects a fixed dev port (see src-tauri/tauri.conf.json).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    target: "es2021",
    outDir: "dist",
    // 两个入口：主窗 index.html 与浮窗 float.html（Tauri 的资源解析不吃
    // `index.html?view=float` 这种带查询串的 URL，只能各占一个真实文件）。
    rollupOptions: {
      input: {
        // 用 import.meta.url 拼绝对路径：这里没有 @types/node，不能用 __dirname。
        main: new URL("index.html", import.meta.url).pathname,
        float: new URL("float.html", import.meta.url).pathname,
      },
    },
  },
});
