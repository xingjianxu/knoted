import { expect, test, type Locator, type Page } from "@playwright/test";

// Check actual portal placement, computed public theme/CSS, and hit-testing, not
// jsdom or just a class name. Positioners/viewport own menu/toast stacking.
async function expectPortal(popup: Locator, layer: Locator, zIndex: string) {
  await expect(popup).toBeVisible();
  await expect(popup).toHaveCSS("font-family", /system-ui/);
  await expect(popup).toHaveCSS("color", "rgb(31, 41, 51)");
  await expect(popup).toHaveCSS("background-color", "rgb(255, 255, 255)");
  await expect(popup).toHaveCSS("border-top-width", "1px");
  await expect(layer).toHaveCSS("z-index", zIndex);
  expect(await popup.evaluate((element) => !document.getElementById("root")?.contains(element))).toBe(true);
  await expect.poll(() => popup.evaluate((element) => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
}

test.beforeEach(async ({ page }) => {
  const errors: string[] = [];
  const external: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (!request.url().startsWith("http://127.0.0.1:4177/")) external.push(request.url());
  });
  await page.goto("/tests/e2e/fixtures/index.html?task=t18");
  await expect(page.getByRole("heading", { name: "T18 弹层消费夹具" })).toBeVisible();
  observers.set(page, { errors, external });
});

const observers = new WeakMap<Page, { errors: string[]; external: string[] }>();
test.afterEach(async ({ page }) => {
  const observed = observers.get(page);
  expect(observed?.errors).toEqual([]);
  expect(observed?.external).toEqual([]);
});

test("t18_1 Dialog traps Tab/Shift+Tab and real portal theme is above its backdrop", async ({ page }) => {
  await page.getByRole("button", { name: "编辑笔记", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "编辑笔记" });
  await expectPortal(dialog, dialog, "1201");
  await expect(page.locator(".mt-backdrop")).toHaveCSS("z-index", "1200");
  await expect(dialog).toHaveAccessibleDescription("测试受控弹层与焦点生命周期");
  await expect(page.getByLabel("受控状态")).toContainText("Dialog：true");
  const close = dialog.getByRole("button", { name: "关闭", exact: true });
  const finish = dialog.getByRole("button", { name: "完成编辑" });
  await close.focus();
  await page.keyboard.press("Shift+Tab");
  await expect(finish).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();
  for (let i = 0; i < 10; i++) {
    await page.keyboard.press("Tab");
    // Base UI briefly focuses an inert guard while wrapping; wait for the
    // redirected user-focusable control, rather than reading that transient.
    await expect.poll(() => dialog.evaluate((element) => element.contains(document.activeElement))).toBe(true);
  }
  for (let i = 0; i < 10; i++) {
    await page.keyboard.press("Shift+Tab");
    await expect.poll(() => dialog.evaluate((element) => element.contains(document.activeElement))).toBe(true);
  }
});

test("t18_2 Dialog Esc/buttons/outside return focus; removed trigger closes safely", async ({ page }) => {
  const trigger = page.getByRole("button", { name: "编辑笔记", exact: true });
  const dialog = page.getByRole("dialog", { name: "编辑笔记" });
  for (const close of ["Escape", "关闭", "完成编辑", "outside"]) {
    await trigger.click();
    await expect(dialog).toBeVisible();
    if (close === "Escape") await page.keyboard.press("Escape");
    else if (close === "outside") await page.mouse.click(8, 8);
    else await dialog.getByRole("button", { name: close, exact: true }).click();
    await expect(dialog).not.toBeVisible();
    await expect(page.getByLabel("受控状态")).toContainText("Dialog：false");
    await expect(trigger).toBeFocused();
  }
  await trigger.click();
  await dialog.getByRole("button", { name: "卸载触发点" }).click();
  await expect(trigger).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(page.getByLabel("受控状态")).toContainText("Dialog：false");
  expect(await page.evaluate(() => document.activeElement?.isConnected)).toBe(true);
  await page.getByRole("button", { name: "页面外部按钮" }).click();
});

test("t18_2 Popover controlled close/Esc restore focus; outside click focuses its target", async ({ page }) => {
  const trigger = page.getByRole("button", { name: "查看信息" });
  const popup = page.getByRole("dialog", { name: "笔记信息" });
  await trigger.click();
  await expectPortal(popup, page.locator(".mt-positioner").filter({ has: popup }), "1250");
  await expect(page.getByLabel("受控状态")).toContainText("Popover：true");
  await popup.getByRole("button", { name: "收起信息" }).click();
  await expect(popup).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await trigger.click();
  await page.keyboard.press("Escape");
  await expect(popup).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await trigger.click();
  const outside = page.getByRole("button", { name: "页面外部按钮" });
  await outside.click();
  await expect(popup).not.toBeVisible();
  await expect(page.getByLabel("受控状态")).toContainText("Popover：false");
  await expect(outside).toBeFocused();
});

for (const source of ["dropdown", "context"] as const) {
  test(`t18_3 ${source} keyboard disabled item is inert and enabled selection runs once`, async ({ page }) => {
    const trigger = page.getByRole("button", { name: source === "dropdown" ? "笔记菜单" : "笔记上下文区域" });
    if (source === "dropdown") {
      await trigger.focus();
      await page.keyboard.press("ArrowDown");
    } else {
      await trigger.click({ button: "right" });
      await page.keyboard.press("ArrowDown");
    }
    const menu = page.getByRole("menu");
    await expectPortal(menu, page.locator(".mt-positioner").filter({ has: menu }), "1250");
    const first = menu.getByRole("menuitem", { name: "打开笔记" });
    const disabled = menu.getByRole("menuitem", { name: "删除笔记" });
    const last = menu.getByRole("menuitem", { name: "归档笔记" });
    await expect(first).toBeFocused();
    await expect(disabled).toHaveAttribute("aria-disabled", "true");
    await disabled.click({ force: true });
    await expect(page.getByLabel("菜单动作")).toHaveText("下拉：0；上下文：0；最后：无");
    // Published menu items remain keyboard-focusable for discoverability.
    // Disabled means non-executable, not skipped by arrow navigation.
    await first.focus();
    await page.keyboard.press("ArrowDown");
    await expect(disabled).toBeFocused();
    await page.keyboard.press("Enter");
    await page.keyboard.press("Space");
    await expect(menu).toBeVisible();
    await expect(page.getByLabel("菜单动作")).toHaveText("下拉：0；上下文：0；最后：无");
    await page.keyboard.press("ArrowDown");
    await expect(last).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(disabled).toBeFocused();
    await page.keyboard.press("ArrowUp");
    await expect(first).toBeFocused();
    await page.keyboard.press("End");
    await expect(last).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(menu).not.toBeVisible();
    await expect(page.getByLabel("菜单动作")).toHaveText(
      source === "dropdown" ? "下拉：1；上下文：0；最后：archive" : "下拉：0；上下文：1；最后：archive",
    );
    await expect(page.getByLabel("受控状态")).toContainText(source === "dropdown" ? "Dropdown：false" : "Context：false");
    if (source === "dropdown") await expect(trigger).toBeFocused();
  });
}

test("t18_4 Confirm cancel is inert; pending prevents repeats and rejection allows successful retry", async ({ page }) => {
  const trigger = page.getByRole("button", { name: "归档确认" });
  const dialog = page.getByRole("alertdialog", { name: "确认归档" });
  await trigger.click();
  await expectPortal(dialog, dialog, "1201");
  await dialog.getByRole("button", { name: "取消" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
  await expect(page.getByLabel("确认结果")).toHaveText("请求：0；成功：0");
  await trigger.click();
  await dialog.getByRole("button", { name: "确认归档", exact: true }).click();
  const pending = dialog.getByRole("button", { name: "…" });
  await expect(pending).toBeDisabled();
  await expect(dialog.getByRole("button", { name: "取消" })).toBeDisabled();
  await pending.click({ force: true });
  await page.keyboard.press("Enter");
  await dialog.getByRole("button", { name: "取消" }).click({ force: true });
  await expect(dialog).toBeVisible();
  await expect(page.getByLabel("确认结果")).toHaveText("请求：1；成功：0");
  await dialog.getByRole("button", { name: "模拟请求失败" }).click();
  await expect(dialog.getByRole("alert")).toHaveText("操作失败，请重试。");
  await expect(dialog).toBeVisible();
  await expect(page.getByLabel("确认结果")).toHaveText("请求：1；成功：0");
  await expect(page.getByLabel("受控状态")).toContainText("Confirm：true");
  await dialog.getByRole("button", { name: "确认归档", exact: true }).click();
  await expect(dialog.getByRole("alert")).toHaveCount(0);
  await expect(pending).toBeDisabled();
  await pending.click({ force: true });
  await expect(page.getByLabel("确认结果")).toHaveText("请求：2；成功：0");
  await dialog.getByRole("button", { name: "模拟请求成功" }).click();
  await expect(dialog).not.toBeVisible();
  await expect(page.getByLabel("确认结果")).toHaveText("请求：2；成功：1");
  await expect(page.getByLabel("受控状态")).toContainText("Confirm：false");
  await expect(trigger).toBeFocused();
});

test("t18_5 StrictMode success/error notifications are single, styled, layered and closable", async ({ page }) => {
  await page.getByRole("button", { name: "成功通知" }).click();
  await page.getByRole("button", { name: "失败通知" }).click();
  const success = page.locator(".mt-toast").filter({ has: page.getByRole("heading", { name: "保存成功" }) });
  const error = page.locator(".mt-toast").filter({ has: page.getByRole("heading", { name: "保存失败" }) });
  await expect(success).toHaveCount(1);
  await expect(error).toHaveCount(1);
  await expect(success).toHaveAttribute("data-type", "success");
  await expect(error).toHaveAttribute("data-type", "error");
  await expect(success).toContainText("笔记已保存");
  await expect(error).toContainText("请重试保存");
  await page.getByRole("button", { name: "编辑笔记", exact: true }).click();
  // Both toast roots must still hit-test above the modal backdrop.
  for (const toast of [success, error]) {
    await expectPortal(toast, page.getByRole("region", { name: "通知", exact: true }), "1300");
  }
  await page.keyboard.press("Escape");
  for (const toast of [success, error]) {
    await toast.hover();
    await toast.getByRole("button", { name: "关闭", exact: true }).click();
    await expect(toast).toHaveCount(0);
  }
  await expect(page.locator(".mt-toast")).toHaveCount(0);
});
