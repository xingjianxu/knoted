import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import App from "../../src/App";
import { createMemoryHistory } from "@tanstack/react-router";
import { createAppRouter } from "../../src/lib/router";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const webRoot = path.resolve(testDir, "../..");
const repoRoot = path.resolve(webRoot, "../..");
const requireWeb = createRequire(import.meta.url);

function read(relativePath: string, root = webRoot): string {
  return readFileSync(path.join(root, relativePath), "utf8");
}

function readSources(directory: string): Array<{ file: string; text: string }> {
  return readdirSync(directory, { recursive: true })
    .filter(
      (entry): entry is string =>
        typeof entry === "string" && /\.(ts|tsx|css)$/.test(entry),
    )
    .sort()
    .map((entry) => ({
      file: path.join(directory, entry),
      text: readFileSync(path.join(directory, entry), "utf8"),
    }));
}

describe("t02 web scaffold", () => {
  it("t02_1_renders_app_shell", async () => {
    const scroll = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    const router = createAppRouter({
      history: createMemoryHistory({ initialEntries: ["/login"] }),
    });
    render(<App router={router} />);
    await screen.findByRole("button", { name: "使用 Passkey 登录" });

    expect(
      screen.getByRole("heading", { level: 1, name: "登录" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Knoted", { exact: true })).toBeInTheDocument();
    scroll.mockRestore();
  });

  it("t02_1_cleanup_between_tests", async () => {
    const scroll = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    // The previous test rendered <App />; afterEach(cleanup) in tests/setup.ts
    // must have removed it, otherwise DOM nodes leak between tests.
    expect(document.querySelectorAll("h1")).toHaveLength(0);

    render(
      <App
        router={createAppRouter({
          history: createMemoryHistory({ initialEntries: ["/login"] }),
        })}
      />,
    );
    render(
      <App
        router={createAppRouter({
          history: createMemoryHistory({ initialEntries: ["/login"] }),
        })}
      />,
    );
    await screen.findAllByRole("button", { name: "使用 Passkey 登录" });

    expect(
      screen.getAllByRole("heading", { level: 1, name: "登录" }),
    ).toHaveLength(2);
    scroll.mockRestore();
  });

  it("t02_1_build_uses_no_remote_fonts", () => {
    const sources = [
      { file: path.join(webRoot, "index.html"), text: read("index.html") },
      ...readSources(path.join(webRoot, "src")),
      {
        file: requireWeb.resolve("@bixlab/mtone/styles.css"),
        text: readFileSync(requireWeb.resolve("@bixlab/mtone/styles.css"), "utf8"),
      },
      {
        file: requireWeb.resolve("@bixlab/mtone-blockeditor/styles.css"),
        text: readFileSync(requireWeb.resolve("@bixlab/mtone-blockeditor/styles.css"), "utf8"),
      },
    ];

    const remoteFontPatterns = [
      /fonts\.googleapis\.com/,
      /fonts\.gstatic\.com/,
      /use\.typekit/,
      /@font-face[^}]*url\(\s*["']?https?:/,
      /@import\s+(?:url\(\s*)?["']?https?:/,
    ];

    for (const source of sources) {
      for (const pattern of remoteFontPatterns) {
        expect(source.text, `${source.file} matches ${pattern}`).not.toMatch(
          pattern,
        );
      }
    }

    const stylesEntry = sources.find(
      (source) => source.file === path.join(webRoot, "src/styles.css"),
    );

    expect(stylesEntry, "src/styles.css was not read").toBeDefined();
    expect(stylesEntry?.text).toMatch(/system-ui/);
  });

  it("t02_1_strict_typescript_enabled", () => {
    const webTsconfig = JSON.parse(read("tsconfig.json")) as {
      compilerOptions?: { strict?: boolean };
      include?: string[];
    };
    const webPackage = JSON.parse(read("package.json")) as {
      scripts?: Record<string, string>;
    };
    const rootPackage = JSON.parse(
      readFileSync(path.join(repoRoot, "package.json"), "utf8"),
    ) as { scripts?: Record<string, string> };

    expect(webTsconfig.compilerOptions?.strict).toBe(true);
    expect(webTsconfig.include).toContain("src");
    expect(webPackage.scripts?.typecheck).toContain("tsc --noEmit");
    expect(rootPackage.scripts?.typecheck).toContain("pnpm");
  });

  it("t02_1_zero_tests_cannot_pass", () => {
    const webPackage = JSON.parse(read("package.json")) as {
      scripts?: Record<string, string>;
    };
    const testScript = webPackage.scripts?.test ?? "";

    expect(testScript).not.toContain("--passWithNoTests");
    expect(read("vite.config.ts")).not.toMatch(/passWithNoTests\s*:\s*true/);
  });
});
