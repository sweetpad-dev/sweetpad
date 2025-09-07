import { sentryEsbuildPlugin } from "@sentry/esbuild-plugin";
import dotenv from "dotenv";
import esbuild from "esbuild";
import { copy } from 'esbuild-plugin-copy';
import fs from "node:fs";
import path from "node:path";


dotenv.config();

const args = process.argv.slice(2);
const isWatch = args.includes("--watch");
const isMinify = args.includes("--minify");
const isSourcemap = args.includes("--sourcemap");
const isProduction = args.includes("--production");

const packageJsonPath = path.resolve(process.cwd(), "package.json");
const pkg = JSON.parse(fs.readFileSync(packageJsonPath, "utf-8"));
const version = pkg.version;

const config = {
  entryPoints: ["./src/extension.ts"],
  bundle: true,
  outfile: "out/extension.js",
  external: ["vscode"],
  format: "cjs",
  platform: "node",
  target: "es6",
  sourcemap: isSourcemap,
  minify: isMinify,
  define: {
    GLOBAL_SENTRY_DSN: JSON.stringify(process.env.SENTRY_DSN ?? null),
    GLOBAL_RELEASE_VERSION: isProduction ? JSON.stringify(version) : JSON.stringify("dev"),
  },
  plugins: [
    {
      // for VSCode $esbuild-watch problem matcher
      name: "esbuild-problem-matcher",
      setup(build) {
        build.onStart(() => {
          console.log("[watch] build started");
        });
        build.onEnd((result) => {
          for (const { text, location } of result.errors) {
            console.error(`✘ [ERROR] ${text}`);
            console.error(`    ${location.file}:${location.line}:${location.column}:`);
          }
          console.log("[watch] build finished");
        });
      },
    },
    copy({
      resolveFrom: 'cwd',
        assets: {
          from: ["./src/debugger/sweetpadlldb.py"],
          to: ['./out/sweetpadlldb.py']
        },
        watch: isWatch,
    })
  ],
};

// Upload source maps to Sentry (only on production build, because it's slow and expensive)
if (isProduction) {
  config.plugins.push(
    sentryEsbuildPlugin({
      org: "yevhenii-hyzyla",
      project: "sweetpad",
      release: version,
      authToken: process.env.SENTRY_AUTH_TOKEN,
      disableInstrumenter: true,
    }),
  );
}

if (isWatch) {
  console.log("[watch] build started");
  esbuild
    .context(config)
    .then((ctx) => {
      ctx.watch();
      console.log("Watching for changes...");
    })
    .catch(() => process.exit(1));
} else {
  esbuild
    .build(config)
    .then(() => {
      console.log("Build completed.");
    })
    .catch(() => process.exit(1));
}
