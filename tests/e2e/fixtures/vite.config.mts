import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import webConfig from "../../../apps/web/vite.config.ts";

// The fixture HTML lives outside the pnpm web workspace. Resolve its React
// optimizer entries from the consuming app, not from the repository root.
const requireWeb = createRequire(new URL("../../../apps/web/package.json", import.meta.url));

export default {
  ...webConfig,
  root: fileURLToPath(new URL("../../..", import.meta.url)),
  server: { host: "127.0.0.1", port: 4177, strictPort: true },
  resolve: {
    alias: [
      { find: /^react(?=\/|$)/, replacement: path.dirname(requireWeb.resolve("react/package.json")) },
      { find: /^react-dom(?=\/|$)/, replacement: path.dirname(requireWeb.resolve("react-dom/package.json")) },
    ],
  },
};
