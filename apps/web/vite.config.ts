import { tanstackRouter } from "@tanstack/router-plugin/vite";
import { defineConfig } from "vitest/config";

import { componentPlugins } from "./vite.components.ts";

export default defineConfig(() => {
  const origin = process.env.KNOTED_OFFICE_ORIGIN ?? "";
  // URL canonicalization alone permits some host characters that aren't safe CSP sources.
  if (origin && (!/^https:\/\/(?:[a-z0-9-]+(?:\.[a-z0-9-]+)*|\[[a-f0-9:]+\])(?::[0-9]+)?$/.test(origin) ||
    URL.parse(origin)?.origin !== origin)) {
    throw new Error("KNOTED_OFFICE_ORIGIN must be a canonical HTTPS origin without credentials, path, query or fragment");
  }
  const officeSource = origin ? ` ${origin}` : "";
  return {
    define: { "import.meta.env.KNOTED_OFFICE_ORIGIN": JSON.stringify(origin) },
    plugins: [
      {
        name: "knoted-preview-csp",
        // Ship the policy in the static app; Vite's hot-reload HTML is not a release artifact.
        apply: "build",
        transformIndexHtml: () => [{
          tag: "meta",
          injectTo: "head-prepend",
          attrs: {
            "http-equiv": "Content-Security-Policy",
            content: `script-src 'self'${officeSource}; frame-src 'self' blob:${officeSource}; object-src 'none'`,
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
