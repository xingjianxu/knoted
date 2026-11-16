import { expect, test, type Page } from "@playwright/test";

// Real Chromium consumption of the published Tree/List/Empty/Spinner/Alert/
// ConfirmDialog composition. Page errors and non-local requests are observed
// for every case; theme/CSS assertions read computed styles rather than jsdom.

const observers = new WeakMap<Page, { errors: string[]; external: string[] }>();

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  const external: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4177/")) external.push(request.url());
  });
  await page.goto("/tests/e2e/fixtures/index.html?task=t19");
  await expect(page.getByRole("heading", { name: "T19 树、游标列表与反馈消费夹具" })).toBeVisible();
  observers.set(page, { errors, external });
});

test.afterEach(async ({ page }) => {
  const observed = observers.get(page);
  expect(observed?.errors).toEqual([]);
  expect(observed?.external).toEqual([]);
});

test("t19_1 tree mouse toggle and keyboard expand without selecting, disabled stays inert", async ({ page }) => {
  const tree = page.getByRole("tree", { name: "笔记树" });
  const project = tree.getByRole("treeitem", { name: "项目", exact: true });
  const selection = page.getByLabel("树选择");
  const expanded = page.getByLabel("树展开");

  await expect(selection).toHaveText("无");
  await project.locator(".mt-tree-toggle").click();
  await expect(project).toHaveAttribute("aria-expanded", "true");
  await expect(expanded).toHaveText("project");
  await expect(selection).toHaveText("无");
  await expect(tree.getByRole("treeitem", { name: "项目甲", exact: true })).toBeVisible();

  // Selected tree item receives the published selected background token.
  const projectA = tree.getByRole("treeitem", { name: "项目甲", exact: true });
  await projectA.click();
  await expect(selection).toHaveText("project-a");
  await expect(projectA).toHaveAttribute("aria-selected", "true");
  await expect(projectA).toHaveCSS("background-color", "rgb(233, 238, 244)");

  // Keyboard: ArrowRight expands, ArrowDown moves to the child, Enter selects.
  await project.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(expanded).toHaveText("无");
  await page.keyboard.press("ArrowRight");
  await expect(project).toHaveAttribute("aria-expanded", "true");
  await expect(selection).toHaveText("project-a");
  await page.keyboard.press("ArrowDown");
  await expect(projectA).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(selection).toHaveText("project-a");

  // Disabled nodes expose aria-disabled and neither select nor expand.
  const locked = tree.getByRole("treeitem", { name: "已锁定", exact: true });
  await expect(locked).toHaveAttribute("aria-disabled", "true");
  await locked.click({ force: true });
  await expect(selection).toHaveText("project-a");
  const projectB = tree.getByRole("treeitem", { name: "项目乙", exact: true });
  await expect(projectB).toHaveAttribute("aria-disabled", "true");
  await projectB.click({ force: true });
  await expect(selection).toHaveText("project-a");
});

test("t19_2 empty/error/loading states are mutually exclusive and recover", async ({ page }) => {
  await expect(page.getByText("首页笔记一")).toBeVisible();
  await page.getByRole("button", { name: "模拟加载" }).click();
  await expect(page.getByRole("status", { name: "加载笔记" })).toBeVisible();
  await expect(page.getByRole("list", { name: "笔记列表" })).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText("没有笔记")).toHaveCount(0);
  await page.getByRole("button", { name: "模拟失败" }).click();
  await expect(page.getByRole("alert")).toContainText("加载失败");
  await expect(page.getByRole("status", { name: "加载笔记" })).toHaveCount(0);
  await expect(page.getByRole("list", { name: "笔记列表" })).toHaveCount(0);
  await expect(page.getByText("没有笔记")).toHaveCount(0);
  await page.getByRole("button", { name: "模拟空列表" }).click();
  await expect(page.getByText("没有笔记")).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("status", { name: "加载笔记" })).toHaveCount(0);
  await expect(page.getByRole("list", { name: "笔记列表" })).toHaveCount(0);
  await page.getByRole("button", { name: "恢复数据" }).click();
  await expect(page.getByText("首页笔记一")).toBeVisible();
});

test("t19_3 confirm object copy, cancel zero callbacks, pending single and visible failure", async ({ page }) => {
  const trigger = page.getByRole("button", { name: "归档项目甲" });
  const dialog = page.getByRole("alertdialog", { name: "确认归档“项目甲”" });
  const result = page.getByLabel("确认结果");

  await trigger.click();
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAccessibleDescription(/项目甲/);
  await expect(dialog).toHaveCSS("font-family", /system-ui/);
  await expect(dialog).toHaveCSS("color", "rgb(31, 41, 51)");
  await dialog.getByRole("button", { name: "取消" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(result).toHaveText("请求：0；成功：0");

  await trigger.click();
  await dialog.getByRole("button", { name: "确认归档", exact: true }).click();
  const pending = dialog.getByRole("button", { name: "…" });
  await expect(pending).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "取消" })).toBeDisabled();
  await pending.click({ force: true });
  await page.keyboard.press("Enter");
  await dialog.getByRole("button", { name: "取消" }).click({ force: true });
  await expect(dialog).toBeVisible();
  await expect(result).toHaveText("请求：1；成功：0");

  await dialog.getByRole("button", { name: "模拟请求失败" }).click();
  await expect(dialog.getByRole("alert")).toHaveText("操作失败，请重试。");
  await expect(dialog).toBeVisible();
  await expect(result).toHaveText("请求：1；成功：0");

  await dialog.getByRole("button", { name: "确认归档", exact: true }).click();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(pending).toBeDisabled();
  await pending.click({ force: true });
  await expect(result).toHaveText("请求：2；成功：0");
  await dialog.getByRole("button", { name: "模拟请求成功" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(result).toHaveText("请求：2；成功：1");
  await expect(trigger).toBeFocused();
});

test("t19_4 cursor paging without total, disabled boundaries and filter reset", async ({ page }) => {
  const previous = page.getByRole("button", { name: "上一页" });
  const next = page.getByRole("button", { name: "下一页" });
  const state = page.getByLabel("列表状态");

  await expect(previous).toBeDisabled();
  await expect(next).toBeEnabled();
  await expect(page.getByText("首页笔记一")).toBeVisible();
  await expect(state).toHaveText("游标：首屏；历史：0；筛选：all");

  // Real list/button theme tokens reach the controls.
  await expect(next).toHaveCSS("font-family", /system-ui/);
  await expect(next).toHaveCSS("color", "rgb(31, 41, 51)");
  await expect(page.getByRole("listitem").first()).toHaveCSS("border-bottom-width", "1px");

  await next.click();
  await expect(page.getByText("第二页笔记三")).toBeVisible();
  await expect(previous).toBeEnabled();
  await expect(state).toHaveText("游标：opaque:二+/=；历史：1；筛选：all");
  await next.click();
  await expect(page.getByText("末页笔记五")).toBeVisible();
  await expect(next).toBeDisabled();
  await expect(state).toHaveText("游标：v3.%2F?甲；历史：2；筛选：all");

  await previous.click();
  await expect(page.getByText("第二页笔记三")).toBeVisible();
  await previous.click();
  await expect(page.getByText("首页笔记一")).toBeVisible();
  await expect(previous).toBeDisabled();

  await next.click();
  await next.click();
  await page.getByRole("button", { name: "筛选星标" }).click();
  await expect(state).toHaveText("游标：首屏；历史：0；筛选：starred");
  await expect(previous).toBeDisabled();
  await expect(page.getByText("首页笔记一")).toBeVisible();
});
