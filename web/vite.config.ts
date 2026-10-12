import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// No analytics, no third-party scripts: everything the page loads is built from this directory.
export default defineConfig({ plugins: [react()], build: { target: "es2022", sourcemap: false, chunkSizeWarningLimit: 900 } });
