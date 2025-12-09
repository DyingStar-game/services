import { defineConfig } from "tsup";

export default defineConfig({
  entry: [
    "src/index.ts",
    "src/scripts/import/import-system.ts",
    "src/scripts/db/reset-db.ts",
  ],
  format: ["esm"],
  dts: false,
  splitting: false,
  sourcemap: true,
  clean: true,
  minify: false,
  target: "node18",
  outDir: "dist",
  tsconfig: "tsconfig.json",
});
