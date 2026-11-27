import { existsSync, readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { cn } from "@bixlab/mtone";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { UiProvider } from "../../src/lib/ui-provider";
import { T17Form } from "../fixtures/t17-form";

const webRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const repoRoot = path.resolve(webRoot, "../..");

function renderForm() {
  return render(
    <UiProvider>
      <T17Form />
    </UiProvider>,
  );
}

describe("t17 mtone consumption", () => {
  it("t17_1 labels identify real controls and controls accept input", () => {
    renderForm();
    const title = screen.getByRole("textbox", { name: "标题" });
    const description = screen.getByRole("textbox", { name: "说明" });
    expect(screen.getByText("测试夹具")).toBeVisible();
    fireEvent.change(title, { target: { value: "知识笔记" } });
    fireEvent.change(description, { target: { value: "上下文" } });
    expect(title).toHaveValue("知识笔记");
    expect(description).toHaveValue("上下文");
    title.focus();
    expect(title).toHaveFocus();
    // Actual Tab order is verified in Chromium, not simulated by jsdom.
  });

  it("t17_2 disabled and loading prevent repeated submit, then recover", () => {
    renderForm();
    const save = screen.getByRole("button", { name: "保存" });
    fireEvent.change(screen.getByLabelText("标题"), { target: { value: "已提交" } });
    fireEvent.click(save);
    expect(screen.getByRole("status")).toHaveTextContent("提交次数：1；标题：已提交");
    fireEvent.click(screen.getByRole("button", { name: "禁用提交" }));
    expect(save).toBeDisabled();
    fireEvent.click(save);
    expect(screen.getByRole("status")).toHaveTextContent("提交次数：1");
    fireEvent.click(screen.getByRole("button", { name: "进入加载" }));
    expect(save).toBeDisabled();
    expect(save).toHaveAttribute("aria-busy", "true");
    fireEvent.click(save);
    expect(screen.getByRole("status")).toHaveTextContent("提交次数：1");
    fireEvent.click(screen.getByRole("button", { name: "恢复提交" }));
    expect(save).toBeEnabled();
    expect(save).not.toHaveAttribute("aria-busy");
    fireEvent.click(save);
    expect(screen.getByRole("status")).toHaveTextContent("提交次数：2");
  });

  it("t17_3 descriptions and errors are associated and recover", () => {
    renderForm();
    const title = screen.getByLabelText("标题");
    expect(title).toHaveAccessibleDescription("填写笔记标题");
    expect(screen.getByLabelText("说明")).toHaveAccessibleDescription("补充上下文");
    fireEvent.click(screen.getByRole("button", { name: "切换错误" }));
    expect(title).toHaveAttribute("aria-invalid", "true");
    expect(title).toHaveAccessibleDescription("标题不可用");
    fireEvent.click(screen.getByRole("button", { name: "切换错误" }));
    // 0.3.0 preserves an unset child aria-invalid in manual Form.Item mode.
    // Missing and "false" both mean valid; assert the accessible semantics.
    expect(title).toBeValid();
    expect(title).toHaveAccessibleDescription("填写笔记标题");
  });

  it("t17_4 removes placeholder sources and directly loads published CSS", () => {
    const pkg = JSON.parse(readFileSync(path.join(webRoot, "package.json"), "utf8"));
    expect(pkg.dependencies["@bixlab/mtone"]).toBe("1.0.0");
    expect(pkg.dependencies["@bixlab/mtone-blockeditor"]).toBe("1.1.0");
    expect(pkg.dependencies).not.toHaveProperty("@knoted/ui");
    expect(pkg.dependencies).not.toHaveProperty("@blocknote/core");
    expect(pkg.dependencies).not.toHaveProperty("@blocknote/react");
    // Old installs may leave ignored node_modules behind. The removal contract
    // concerns the workspace manifest and sources, not package-manager caches.
    expect(existsSync(path.join(repoRoot, "packages/ui/package.json"))).toBe(false);
    expect(existsSync(path.join(repoRoot, "packages/ui/src"))).toBe(false);
    const sourceFiles = readdirSync(path.join(webRoot, "src"), { recursive: true })
      .filter((file): file is string => typeof file === "string" && /\.(tsx?|css)$/.test(file));
    const sources = sourceFiles
      .map((file) => readFileSync(path.join(webRoot, "src", file), "utf8"))
      .join("\n");
    expect(sources).not.toContain("@knoted/ui");
    expect(sources).not.toContain("@blocknote/");
    expect(sources).toContain('@import "@bixlab/mtone/styles.css"');
    expect(sources).toContain('@import "@bixlab/mtone-blockeditor/styles.css"');
  });

  it("characterizes mtone cn: joins but does not merge conflicting utilities", () => {
    // Old @knoted/ui cn used tailwind-merge: cn('p-2', 'p-4') === 'p-4'.
    // App has static classes, so it needs neither helper after migration.
    expect(cn("p-2", false, undefined, "p-4")).toBe("p-2 p-4");
  });
});
