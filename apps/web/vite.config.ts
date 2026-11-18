import { fileURLToPath } from "node:url";
import { tanstackRouter } from "@tanstack/router-plugin/vite";
import { defineConfig } from "vitest/config";

import { componentPlugins } from "./vite.components.ts";

export default defineConfig(({ command, mode }) => {
  const origin = process.env.KNOTED_OFFICE_ORIGIN ?? "";
  // URL canonicalization alone permits some host characters that aren't safe CSP sources.
  if (origin && (!/^https:\/\/[a-z0-9-]+(?:\.[a-z0-9-]+)*(?::[0-9]+)?$/.test(origin) ||
    URL.parse(origin)?.origin !== origin)) {
    throw new Error("KNOTED_OFFICE_ORIGIN must be a canonical HTTPS DNS/IPv4 origin without credentials, path, query or fragment");
  }
  const officeSource = origin ? ` ${origin}` : "";
  return {
    // Development HMR HTML does not have the release policies; do not enable Office there.
    define: { "import.meta.env.KNOTED_OFFICE_ORIGIN": JSON.stringify(command === "build" || mode === "test" ? origin : "") },
    build: {
      rolldownOptions: {
        input: ["index.html", "office-preview.html"].map(path => fileURLToPath(new URL(path, import.meta.url))),
      },
    },
    plugins: [
      {
        name: "knoted-preview-csp",
        // Ship the policy in the static app; Vite's hot-reload HTML is not a release artifact.
        apply: "build",
        transformIndexHtml: (_html, context) => [{

          tag: "meta",
          injectTo: "head-prepend",
          attrs: {
            "http-equiv": "Content-Security-Policy",
            content: context.path === "/office-preview.html"
              ? `script-src 'self'${officeSource}; frame-src ${origin || "'none'"}; object-src 'none'`
              : "script-src 'self'; frame-src 'self' blob:; object-src 'none'",
          },
        }],
      },
      tanstackRouter({ target: "react", autoCodeSplitting: true }),
      ...componentPlugins(),
    ],
    test: {
      environment: "jsdom",
      setupFiles: ["./tests/setup.ts"],
      include: ["tests/**/*.test.{ts,tsx}"],
    },
  };
});
