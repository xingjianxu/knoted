import { expect, test, type Page } from "@playwright/test";

// Exercise the real application and CSS; keep the test independent of a live
// account, database and Passkey authenticator by intercepting the API only.
test.use({ baseURL: "http://127.0.0.1:4178" });

const now = "2026-11-19T00:00:00Z";
const fixtureId = (index: number) => `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`;
const chat = { id: fixtureId(40), title: "布局验收会话", provider: "local", version: 1, created_at: now, updated_at: now };
const targetMessageId = fixtureId(62);

async function mockWorkspace(page: Page, authenticated = true) {
  await page.route("**/api/v1/**", (route) => {
    switch (new URL(route.request().url()).pathname) {
      case "/api/v1/auth/session":
        return authenticated ? route.fulfill({ json: {
          session: {
            id: fixtureId(0), user_id: fixtureId(1), current: true,
            created_at: now, last_seen_at: now, idle_expires_at: "2026-11-26T00:00:00Z",
            absolute_expires_at: "2026-12-19T00:00:00Z",
          },
          csrf_token: "synthetic-csrf-for-layout-test",
        } }) : route.fulfill({ status: 401, json: { code: "unauthorized", message: "未登录" } });
      case "/api/v1/dashboard":
        return route.fulfill({ json: {
          counts: { notes: 10, tasks_open: 0, tasks_due_today: 0, projects_active: 0, files: 0 },
          tasks_by_status: { todo: 0, in_progress: 0, done: 0, cancelled: 0 },
          recent_tasks: [], due_tasks: [], job_failures: 0, indexing_enabled: false,
          recent_notes: Array.from({ length: 10 }, (_, index) => ({
            id: fixtureId(index + 10), parent_id: null, title: `最近笔记 ${index + 1}`,
            document: { blocks: [] }, format_version: 1, content_version: 1, position: index,
            created_at: now, updated_at: now, deleted_at: null,
          })),
        } });
      case "/api/v1/trash":
        return route.fulfill({ json: {
          items: Array.from({ length: 20 }, (_, index) => ({
            id: fixtureId(index + 20), entity_type: "task", title: `回收事项 ${index + 1}`,
            deleted_at: now, parent_id: null, version: 1, can_restore: true, reason: null,
          })),
          next_cursor: null,
        } });
      case "/api/v1/tags":
      case "/api/v1/projects":
        return route.fulfill({ json: { items: [], next_cursor: null } });
      case "/api/v1/chat/sessions":
        return route.fulfill({ json: { items: [chat], next_cursor: null } });
      case `/api/v1/chat/sessions/${chat.id}`:
        return route.fulfill({ json: { ...chat, messages: Array.from({ length: 26 }, (_, index) => ({
          id: fixtureId(index + 42), session_id: chat.id, role: index % 2 === 0 ? "user" : "assistant",
          body: `验收消息 ${index + 1}`, run_id: fixtureId(100 + Math.floor(index / 2)), incomplete: false, created_at: now,
        })) } });
      case "/api/v1/search":
        return route.fulfill({ json: { items: [{
          entity_type: "chat", id: targetMessageId, session_id: chat.id, title: "目标问答消息",
          snippet: "验收消息 21", updated_at: now, path: null, score: null,
        }], next_cursor: null } });
      default:
        throw new Error(`Unexpected request: ${route.request().method()} ${route.request().url()}`);
    }
  });
}

test("窄屏图标导航可通过键盘打开页面，并可在桌面折叠或展开", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 700 });
  await mockWorkspace(page);
  await page.goto("/");

  await expect(page.getByRole("heading", { name: "工作概览" })).toBeVisible();
  const expand = page.getByRole("button", { name: "展开侧边栏" });
  await expect(expand).toHaveAttribute("aria-expanded", "false");
  const nav = page.getByRole("navigation", { name: "主导航" });
  await expect(nav.getByRole("button", { name: "概览" })).toHaveAttribute("aria-current", "page");
  const trash = nav.getByRole("button", { name: "回收站" });
  await trash.focus();
  await page.keyboard.press("Enter");
  await expect(page).toHaveURL(/\/trash$/);
  await expect(trash).toHaveAttribute("aria-current", "page");
  await expect(page.getByRole("main")).toHaveCount(1);
  await expect(page.getByRole("heading", { name: "回收站", exact: true })).toBeVisible();
  expect(await page.getByRole("main").evaluate((el) => el.scrollWidth === el.clientWidth)).toBe(true);

  await page.setViewportSize({ width: 1024, height: 700 });
  const collapse = page.getByRole("button", { name: "收起侧边栏" });
  await expect(collapse).toHaveAttribute("aria-expanded", "true");
  await collapse.click();
  await expect(expand).toHaveAttribute("aria-expanded", "false");
  await expand.click();
  await expect(collapse).toHaveAttribute("aria-expanded", "true");
});

test("主区独立滚动并在切换到另一个长页面时归顶", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 480 });
  await mockWorkspace(page);
  await page.goto("/trash");
  const main = page.getByRole("main");
  const logout = page.getByRole("banner").getByRole("button", { name: "退出登录" });
  const nav = page.getByRole("navigation", { name: "主导航" });
  // Warm the destination first: a short loading state would clamp scrollTop
  // to zero even if navigation did not reset the scroll container.
  await expect(page.getByRole("heading", { name: "回收事项 20", exact: true })).toBeAttached();
  await nav.getByRole("button", { name: "概览" }).click();
  await expect(main.getByRole("button", { name: "打开笔记 最近笔记 10" })).toBeAttached();
  await main.evaluate((el) => { el.scrollTop = 320; });
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(320);
  await expect(logout).toBeInViewport();
  expect(await page.evaluate(() => window.scrollY)).toBe(0);

  await nav.getByRole("button", { name: "回收站" }).click();
  await expect(page.getByRole("heading", { name: "回收事项 20", exact: true })).toBeAttached();
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(page.getByRole("heading", { name: "回收站", exact: true })).toBeInViewport();
});

test("搜索跳转到已缓存的问答消息时目标留在视口内", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 640 });
  await mockWorkspace(page);
  await page.goto(`/chat/${chat.id}`);
  const message = page.locator(`[data-message-id="${targetMessageId}"]`);
  await expect(message).toBeAttached();
  await expect(message).not.toBeInViewport();

  await page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: "搜索" }).click();
  await page.getByRole("searchbox", { name: "搜索词" }).fill("验收消息");
  await page.getByRole("search", { name: "搜索资料" }).getByRole("button", { name: "搜索" }).click();
  await page.getByRole("link", { name: "目标问答消息" }).click();
  await expect(page).toHaveURL((url) => url.pathname === `/chat/${chat.id}` && url.searchParams.get("message") === targetMessageId);
  await expect(message).toBeFocused();
  await expect(message).toBeInViewport({ ratio: 0.8 });
});

test("未认证用户只看到独立登录页", async ({ page }) => {
  await mockWorkspace(page, false);
  await page.goto("/");
  await expect(page.getByRole("button", { name: "使用 Passkey 登录" })).toBeVisible();
  await expect(page).toHaveURL(/\/login\?/);
  await expect(page.getByRole("navigation", { name: "主导航" })).toHaveCount(0);
  await expect(page.getByRole("main")).toHaveCount(1);
});
