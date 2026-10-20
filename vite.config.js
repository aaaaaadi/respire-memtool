import { defineConfig } from "vite";

// Tauri 开发约定端口 1420
export default defineConfig({
  // 相对路径资源引用：Tauri 生产环境（tauri://localhost）下绝对路径 /assets 可能加载失败导致黑屏
  base: "./",
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    target: "es2021",
    outDir: "dist",
    assetsDir: "assets",
    rollupOptions: {
      input: {
        main: "index.html",
        selection: "selection.html",
      },
    },
  },
});
