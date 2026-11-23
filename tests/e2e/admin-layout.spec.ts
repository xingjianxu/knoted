import { expect, test, type Page } from "@playwright/test";

// Exercise the real application and CSS; keep the test independent of a live
// account, database and Passkey authenticator by intercepting the API only.
test.use({ baseURL: "http://127.0.0.1:4178" });

const now = "2026-11-19T00:00:00Z";
const fixtureId = (index: number) => `00000000-0000-4000-8000-${String(index).padStart(12, "0")}`;
const chat = { id: fixtureId(40), title: "布局验收会话", provider: "local", version: 1, created_at: now, updated_at: now };
const targetMessageId = fixtureId(62);

async function mockWorkspace(page: Page, authenticated = true) {
  let signedIn = authenticated;
  await page.route("**/api/v1/**", (route) => {
    switch (new URL(route.request().url()).pathname) {
      case "/api/v1/auth/session":
        return signedIn ? route.fulfill({ json: {
          session: {
            id: fixtureId(0), user_id: fixtureId(1), current: true,
            created_at: now, last_seen_at: now, idle_expires_at: "2026-11-26T00:00:00Z",
            absolute_expires_at: "2026-12-19T00:00:00Z",
          },
          csrf_token: "synthetic-csrf-for-layout-test",
        } }) : route.fulfill({ status: 401, json: { code: "unauthorized", message: "未登录" } });
      case "/api/v1/auth/logout":
        signedIn = false;
        return route.fulfill({ status: 204 });
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
      case "/api/v1/auth/passkeys":
      case "/api/v1/auth/sessions":
      case "/api/v1/projects":
      case "/api/v1/tasks":
      case "/api/v1/files":
        return route.fulfill({ json: { items: [], next_cursor: null } });
      case `/api/v1/projects/${fixtureId(201)}`:
      case `/api/v1/tasks/${fixtureId(201)}`:
      case `/api/v1/files/${fixtureId(201)}`:
        return route.fulfill({ status: 404, json: { code: "not_found", message: "不存在" } });
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

for (const width of [1440, 768, 390, 320]) {
  test(`侧栏品牌区与顶栏底边在 ${width}px 对齐`, async ({ page }) => {
    await page.setViewportSize({ width, height: 720 });
    await mockWorkspace(page);
    await page.goto(`/projects/${fixtureId(201)}`);

    const sidebarToggle = page.getByRole("button", {
      name: width < 768 ? "展开侧边栏" : "收起侧边栏",
    });
    await expect(sidebarToggle).toHaveAttribute("aria-expanded", String(width >= 768));
    const header = page.getByRole("banner");
    await expect(header.getByRole("link", { name: "返回项目" })).toBeInViewport({ ratio: 1 });
    await expect(header.getByRole("button", {
      name: width < 768 ? "更多操作" : "账号菜单",
    })).toBeInViewport({ ratio: 1 });

    const brand = page.locator(".mt-sidebar-heading");
    await expect.poll(async () => {
      const [left, right] = await Promise.all([brand.boundingBox(), header.boundingBox()]);
      return left && right ? (right.y + right.height) - (left.y + left.height) : null;
    }).toBe(0);
  });
}

test("桌面顶栏可打开全局搜索、通知和账号设置", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 720 });
  await mockWorkspace(page);
  await page.goto("/");
  const header = page.getByRole("banner");
  const location = header.getByRole("navigation", { name: "当前位置" });
  await expect(location.locator('[aria-current="page"]')).toHaveText("概览");
  await expect(header.getByRole("button", { name: "更多操作" })).toBeHidden();

  await header.getByRole("button", { name: "全局搜索" }).click();
  await expect(page).toHaveURL((url) => url.pathname === "/search");
  await expect(location.locator('[aria-current="page"]')).toHaveText("搜索");

  const notifications = header.getByRole("button", { name: "通知" });
  await notifications.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "通知" })).toContainText("暂无通知");
  await page.keyboard.press("Escape");
  await expect(notifications).toBeFocused();

  await header.getByRole("button", { name: "账号设置" }).click();
  await expect(page.getByRole("heading", { name: "凭据与会话", level: 1 })).toBeVisible();
});

test("320px 顶栏菜单可用键盘访问同一组操作", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  await mockWorkspace(page);
  await page.goto("/");
  const header = page.getByRole("banner");
  await expect(header.getByRole("button", { name: "全局搜索" })).toBeHidden();
  await expect(header.getByRole("button", { name: "账号菜单" })).toBeHidden();

  const more = header.getByRole("button", { name: "更多操作" });
  await more.focus();
  await page.keyboard.press("Enter");
  const menu = page.getByRole("menu", { name: "更多操作" });
  await expect(menu.getByRole("menuitem")).toHaveText(["全局搜索", "通知", "账号设置", "退出登录"]);
  await expect(menu.getByRole("menuitem", { name: "全局搜索" })).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "通知" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "通知" })).toContainText("暂无通知");
  await page.keyboard.press("Escape");
  await expect(more).toBeFocused();

  await page.keyboard.press("Enter");
  await expect(menu.getByRole("menuitem", { name: "全局搜索" })).toBeFocused();
  await page.keyboard.press("End");
  await expect(menu.getByRole("menuitem", { name: "退出登录" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "使用 Passkey 登录" })).toBeVisible();
});

test("320px 手动展开侧栏后，详情返回与顶栏菜单仍可点击", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  await mockWorkspace(page);
  await page.goto(`/files/${fixtureId(201)}`);
  await page.getByRole("button", { name: "展开侧边栏" }).click();

  const banner = page.getByRole("banner");
  const back = banner.getByRole("link", { name: "返回文件列表" });
  const more = banner.getByRole("button", { name: "更多操作" });
  await expect(back).toBeInViewport({ ratio: 1 });
  await expect(more).toBeInViewport({ ratio: 1 });
  const [backBox, moreBox] = await Promise.all([back.boundingBox(), more.boundingBox()]);
  expect(backBox!.x + backBox!.width).toBeLessThan(moreBox!.x);

  await more.click();
  await expect(page.getByRole("menu", { name: "更多操作" })).toBeVisible();
  await page.keyboard.press("Escape");
  await back.click();
  await expect(page).toHaveURL(/\/files$/);
});

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

test("短视口可逐个 Tab 到所有收起的导航项", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 280 });
  await mockWorkspace(page);
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "主导航" });
  await expect(nav.getByRole("button", { name: "概览" })).toHaveAttribute("aria-current", "page");
  await page.getByRole("link", { name: "跳到主要内容" }).focus();
  for (const name of ["概览", "笔记", "项目", "事项", "回收站", "文件", "搜索", "问答", "凭据与会话"]) {
    await page.keyboard.press("Tab");
    const item = nav.getByRole("button", { name });
    await expect(item).toBeFocused();
    await expect(item).toBeInViewport();
    await expect(item).toHaveCSS("outline-style", "solid");
  }
});

test("主区独立滚动并在切换到另一个长页面时归顶", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 480 });
  await mockWorkspace(page);
  await page.goto("/trash");
  const main = page.getByRole("main");
  const account = page.getByRole("banner").getByRole("button", { name: "账号菜单" });
  const nav = page.getByRole("navigation", { name: "主导航" });
  // Warm the destination first: a short loading state would clamp scrollTop
  // to zero even if navigation did not reset the scroll container.
  await expect(page.getByRole("heading", { name: "回收事项 20", exact: true })).toBeAttached();
  await nav.getByRole("button", { name: "概览" }).click();
  await expect(main.getByRole("button", { name: "打开笔记 最近笔记 10" })).toBeAttached();
  await main.evaluate((el) => { el.scrollTop = 320; });
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(320);
  await expect(account).toBeInViewport();
  expect(await page.evaluate(() => window.scrollY)).toBe(0);

  await nav.getByRole("button", { name: "回收站" }).click();
  await expect(page.getByRole("heading", { name: "回收事项 20", exact: true })).toBeAttached();
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(0);
  await expect(page.getByRole("heading", { name: "回收站", exact: true })).toBeInViewport();

  // Trash retains its cached rows while the dashboard deliberately hides
  // stale results during refetch. Restore the entry whose rows stay mounted.
  await main.evaluate((el) => { el.scrollTop = 240; });
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(240);
  await nav.getByRole("button", { name: "概览" }).click();
  await expect(main.getByRole("button", { name: "打开笔记 最近笔记 10" })).toBeAttached();
  await page.goBack();
  // Trash keeps its rows but briefly inserts a refresh spinner above them;
  // scroll anchoring may adjust the restored offset when that line disappears.
  await expect.poll(() => main.evaluate((el) => Math.abs(el.scrollTop - 240))).toBeLessThan(40);
  await page.goForward();
  await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(0);
});

for (const resize of [false, true]) {
  test(`概览${resize ? "视口缩短" : "重新取数"}后恢复历史滚动位置`, async ({ page }) => {
    const viewport = { width: 1024, height: 480 };
    await page.setViewportSize(viewport);
    await mockWorkspace(page);
    let releaseDashboard: () => void = () => {};
    let holdDashboard = false;
    const pending = new Promise<void>((resolve) => { releaseDashboard = resolve; });
    await page.route("**/api/v1/dashboard", async (route) => {
      if (holdDashboard) await pending;
      await route.fallback();
    });
    await page.goto("/");
    const main = page.getByRole("main");
    const overview = main.getByRole("heading", { name: "工作概览" });
    await expect(overview).toBeVisible();
    const maxScroll = await main.evaluate((el) => el.scrollHeight - el.clientHeight);
    await main.evaluate((el) => new Promise<void>((resolve) => {
      el.addEventListener("scroll", () => resolve(), { once: true });
      el.scrollTop = 320;
    }));
    await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(320);
    await page.getByRole("button", { name: "回收站" }).click();
    await expect(main.getByRole("heading", { name: "回收事项 20", exact: true })).toBeAttached();
    if (resize) await page.setViewportSize({ ...viewport, height: viewport.height + maxScroll - 160 });
    holdDashboard = true;
    try {
      await page.goBack();
      await expect(overview).toHaveCount(0);
    } finally {
      releaseDashboard();
    }
    await expect(overview).toBeVisible();
    if (resize) {
      expect(await main.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeLessThan(320);
      await page.setViewportSize(viewport);
    }
    await expect.poll(() => main.evaluate((el) => el.scrollTop)).toBe(320);
  });
}

test("搜索跳转到已缓存的问答消息时目标留在视口内", async ({ page }) => {
  await page.setViewportSize({ width: 1024, height: 640 });
  await mockWorkspace(page);
  await page.goto(`/chat/${chat.id}`);
  const message = page.locator(`[data-message-id="${targetMessageId}"]`);
  await expect(message).toBeAttached();
  await expect(message).not.toBeInViewport();

  await page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: "搜索" }).click();
  await page.getByRole("searchbox", { name: "搜索词" }).fill("验收消息");
  const searchButton = page.getByRole("search", { name: "搜索资料" }).getByRole("button", { name: "搜索" });
  await expect(searchButton).toHaveCSS("background-color", "rgb(55, 53, 47)");
  await searchButton.click();
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

for (const [path, label] of [["projects", "项目"], ["tasks", "事项"]]) {
  test(`${label}详情在 320px 可通过顶栏返回清单`, async ({ page }) => {
    await page.setViewportSize({ width: 320, height: 640 });
    await mockWorkspace(page);
    await page.goto(`/${path}/${fixtureId(201)}`);
    const banner = page.getByRole("banner");
    const back = banner.getByRole("link", { name: `返回${label}` });
    await expect(back).toBeInViewport();
    await expect(back).not.toHaveAttribute("aria-current");
    await expect(banner.getByRole("button", { name: "更多操作" })).toBeInViewport();
    await expect(page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: label }))
      .toHaveAttribute("aria-current", "page");
    await back.focus();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`/${path}$`));
    await expect(back).toHaveCount(0);
  });
}

test("跳过导航后焦点进入工作区主体", async ({ page }) => {
  await mockWorkspace(page);
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "工作概览" })).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "跳到主要内容" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("main")).toBeFocused();
  await expect(page).toHaveURL(/\/$/);
});

test("根加载失败可重试，等待与再次失败时不丢键盘焦点", async ({ page }) => {
  await mockWorkspace(page);
  let attempts = 0;
  let finishFailure: () => void = () => {};
  const pending = new Promise<void>((resolve) => { finishFailure = resolve; });
  await page.route("**/api/v1/auth/session", async (route) => {
    attempts++;
    if (attempts === 2) await pending;
    if (attempts < 3) return route.fulfill({ status: 503, json: { code: "unavailable", message: "不可用" } });
    return route.fallback();
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "无法加载页面" })).toBeVisible();
  const retry = page.getByRole("button", { name: "重试", exact: true });
  await retry.focus();
  await page.keyboard.press("Enter");
  try {
    await expect(page.getByRole("status")).toHaveText("正在重试…");
    await expect(retry).toBeFocused();
    await expect(retry).toBeDisabled();
  } finally {
    finishFailure();
  }
  await expect(retry).toBeEnabled();
  await expect(retry).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "工作概览" })).toBeVisible();
});

test("404 用真实的首页链接退出错误页", async ({ page }) => {
  await mockWorkspace(page);
  await page.goto("/not-a-route");
  await expect(page.getByRole("main").getByRole("heading", { name: "页面不存在" })).toBeVisible();
  const home = page.getByRole("link", { name: "返回首页" });
  await expect(home).toHaveAttribute("href", "/");
  await home.click();
  await expect(page.getByRole("heading", { name: "工作概览" })).toBeVisible();
});
