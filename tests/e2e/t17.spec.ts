import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/tests/e2e/fixtures/index.html");
});

test("t17_1 label, Tab order, keyboard submit and real control styles", async ({ page }) => {
  const title = page.getByRole("textbox", { name: "标题" });
  const description = page.getByRole("textbox", { name: "说明" });
  const save = page.getByRole("button", { name: "保存", exact: true });
  await page.keyboard.press("Tab");
  await expect(title).toBeFocused();
  await title.fill("浏览器笔记");
  await page.keyboard.press("Tab");
  await expect(description).toBeFocused();
  await description.fill("键盘测试");
  await page.keyboard.press("Tab");
  await expect(save).toBeFocused();
  await expect(save).toHaveCSS("outline-style", "solid");
  await expect(save).toHaveCSS("outline-width", "2px");
  // Grid items blockify inline-flex to flex in computed styles.
  await expect(save).toHaveCSS("display", "flex");
  await expect(save).toHaveCSS("height", "28px");
  await expect(title).toHaveCSS("border-top-width", "1px");
  await expect(description).toHaveCSS("min-height", "72px");
  await expect(page.getByText("测试夹具", { exact: true })).toHaveCSS("border-radius", "3px");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("status")).toHaveText("提交次数：1；标题：浏览器笔记");
});

test("t17_2 disabled and loading skip Tab and prevent pointer/keyboard submits", async ({ page }) => {
  const save = page.getByRole("button", { name: "保存", exact: true });
  const description = page.getByRole("textbox", { name: "说明" });
  for (const mode of ["禁用提交", "进入加载"]) {
    await page.getByRole("button", { name: mode }).click();
    await expect(save).toBeDisabled();
    if (mode === "进入加载") await expect(save).toHaveAttribute("aria-busy", "true");
    await save.click({ force: true });
    await description.focus();
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "禁用提交" })).toBeFocused();
    // Enter in a single-line field must not bypass the disabled submit button.
    await page.getByRole("textbox", { name: "标题" }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("status")).toContainText("提交次数：0");
  }
  await page.getByRole("button", { name: "恢复提交" }).click();
  await expect(save).toBeEnabled();
  await expect(save).not.toHaveAttribute("aria-busy");
  await save.focus();
  await page.keyboard.press("Space");
  await expect(page.getByRole("status")).toContainText("提交次数：1");
});

test("t17_3 descriptions and errors stay accessible through recovery", async ({ page }) => {
  const title = page.getByRole("textbox", { name: "标题" });
  await expect(title).toHaveAccessibleDescription("填写笔记标题");
  await expect(page.getByRole("textbox", { name: "说明" })).toHaveAccessibleDescription("补充上下文");
  await page.getByRole("button", { name: "切换错误" }).click();
  await expect(title).toHaveAttribute("aria-invalid", "true");
  await expect(title).toHaveAccessibleDescription("标题不可用");
  await page.getByRole("button", { name: "切换错误" }).click();
  // Manual FormField may omit aria-invalid when the error is cleared.
  await expect(title).not.toHaveAttribute("aria-invalid", "true");
  await expect(title).toHaveAccessibleDescription("填写笔记标题");
});

test("t17_4 application system fonts and theme reach body-level scopes without remote requests", async ({ page }) => {
  const external: string[] = [];
  const errors: string[] = [];
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4177/")) external.push(request.url());
  });
  page.on("pageerror", (error) => errors.push(error.message));
  await page.reload();
  const save = page.getByRole("button", { name: "保存", exact: true });
  await expect(save).toHaveCSS("font-family", /system-ui/);
  await expect(page.getByRole("textbox", { name: "标题" })).toHaveCSS("font-family", /system-ui/);
  // A body-level MtoneScope is rendered by the fixture to model portal inheritance.
  const scope = page.getByRole("note", { name: "body-level scope" });
  await expect(scope).toHaveCSS("font-family", /system-ui/);
  await expect(scope).toHaveCSS("color", "rgb(31, 41, 51)");
  await expect(save).toHaveCSS("color", "rgb(31, 41, 51)");
  expect(external).toEqual([]);
  expect(errors).toEqual([]);
});
