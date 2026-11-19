import { expect, test } from "@playwright/test";

test.use({ baseURL: "http://127.0.0.1:4178" });

const now = "2026-11-19T00:00:00Z";
const session = {
  id: "fixture-session", user_id: "fixture-user", current: true,
  created_at: now, last_seen_at: now,
  idle_expires_at: "2026-11-26T00:00:00Z", absolute_expires_at: "2026-12-19T00:00:00Z",
};
const longName = "这是用于验证狭窄安全页面与确认窗口自动换行的测试凭据".repeat(3);

test("登录取消后保持主操作焦点且可重试", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 560 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "credentials", { value: {
      get: async () => { throw new DOMException("Fixture cancellation", "NotAllowedError"); },
    } });
  });
  let optionCalls = 0;
  let releaseOptions: () => void = () => {};
  const pending = new Promise<void>((resolve) => { releaseOptions = resolve; });
  await page.route("**/api/v1/auth/login/options", async (route) => {
    optionCalls++;
    await pending;
    await route.fulfill({ json: { ceremony_id: "fixture", options: { publicKey: { challenge: "AQID" } } } });
  });
  await page.goto("/login");
  const button = page.getByRole("button", { name: "使用 Passkey 登录" });
  await expect(page.getByRole("heading", { name: "登录", level: 1 })).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(button).toBeFocused();
  await page.keyboard.press("Enter");
  try {
    await expect(button).toBeDisabled();
    await expect(button).toBeFocused();
    await page.keyboard.press("Enter");
  } finally {
    releaseOptions();
  }
  await expect(page.getByRole("alert")).toContainText("认证已取消");
  await expect(button).toBeEnabled();
  await expect(button).toBeFocused();
  expect(optionCalls).toBe(1);
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "首次注册" })).toBeFocused();
});

test("恢复在提交前说明后果，错误保留可修正输入", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 420 });
  await page.route("**/api/v1/auth/recovery/options", (route) => route.fulfill({
    status: 422, json: { code: "invalid_request", message: "合成验证失败" },
  }));
  await page.goto("/recovery");
  await expect(page.getByRole("status")).toContainText("撤销所有旧凭据与会话");
  const token = page.getByLabel("一次性令牌");
  await expect(token).toHaveAttribute("type", "password");
  await token.fill("fixture");
  const name = page.getByRole("textbox", { name: "凭据名称" });
  await name.fill("");
  const submit = page.getByRole("button", { name: "替换凭据并恢复" });
  await submit.click();
  await expect(name).toHaveAttribute("aria-invalid", "true");
  await name.fill("测试凭据");
  await submit.click();
  await expect(page.getByRole("alert")).toContainText("认证信息无效");
  expect((await token.inputValue()).length).toBeGreaterThan(0);
  await expect(name).toHaveValue("测试凭据");
  await expect(page.getByRole("link", { name: "返回登录" })).toBeVisible();
});

test("安全页长凭据和确认框不撑宽主体，取消回到该记录", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  let deletes = 0;
  await page.route("**/api/v1/auth/**", (route) => {
    if (route.request().method() === "DELETE") deletes++;
    switch (new URL(route.request().url()).pathname) {
      case "/api/v1/auth/session":
        return route.fulfill({ json: { session, csrf_token: "synthetic-auth-layout-fixture" } });
      case "/api/v1/auth/passkeys":
        return route.fulfill({ json: { items: [longName, "备用设备"].map((name, i) => ({
          id: `fixture-${i}`, name, created_at: now, last_used_at: null, credential_id: "synthetic", transports: [],
        })), next_cursor: null } });
      case "/api/v1/auth/sessions":
        return route.fulfill({ json: { items: [session, { ...session, id: "fixture-other", current: false }], next_cursor: null } });
      default:
        return route.fulfill({ status: 404, json: { code: "not_found", message: "不存在" } });
    }
  });
  await page.goto("/settings/security");
  await expect(page.getByRole("heading", { name: "凭据与会话", level: 1 })).toBeVisible();
  const main = page.getByRole("main");
  const trigger = page.getByRole("button", { name: `删除 ${longName}`, exact: true });
  await expect(trigger).toBeVisible();
  await expect(main.getByText("当前会话", { exact: true })).toBeVisible();
  expect(await main.evaluate((el) => el.scrollWidth)).toBe(await main.evaluate((el) => el.clientWidth));
  await trigger.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("alertdialog", { name: `删除凭据 ${longName}` });
  const cancel = dialog.getByRole("button", { name: "取消" });
  await expect(cancel).toBeFocused();
  await expect(cancel).toBeInViewport();
  await expect(dialog.getByRole("button", { name: "确认删除" })).toBeInViewport();
  await page.keyboard.press("Escape");
  await expect(dialog).not.toBeVisible();
  await expect(trigger).toBeFocused();
  expect(deletes).toBe(0);
});
