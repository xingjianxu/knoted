import { expect, test, type Locator, type Page } from "@playwright/test";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const id = (number: number) => `00000000-0000-4000-8000-${String(number).padStart(12, "0")}`;
const now = "2026-11-19T00:00:00Z";
const timestamps = { created_at: now, updated_at: now };
const list = <T,>(items: T[], next_cursor: string | null = null) => ({ items, next_cursor });
const project = { id: id(4), title: "调研项目", description: "整理公开材料与调研记录。", version: 1, archived_at: null, deleted_at: null, ...timestamps };
const task = { id: id(5), title: "校对调研报告", description: "复核数据和引用。", project_id: project.id, parent_id: null, version: 1,
  status: "todo", priority: "high", start: null, due: { date: "2026-11-19", time: "17:30", timezone: "Asia/Shanghai" }, archived_at: null, deleted_at: null, ...timestamps };
const document = { blocks: Array.from({ length: 14 }, (_, i) => ({
  id: id(i + 100), type: "paragraph", content: [{ type: "text", text: `调研记录第 ${i + 1} 段，记录公开材料、复核过程及后续行动。`, styles: {} }],
})) };
const note = { id: id(6), title: "调研手记", document, content_version: 2, format_version: 1,
  parent_id: null, position: 0, deleted_at: null, ...timestamps };
const file = { id: id(7), name: "调研结果.txt", relative_path: "资料/项目/调研结果.txt", note: "校对后发布。", size: 280,
  mime: "text/plain", tag_ids: [], sha256: "b".repeat(64), version: 1, content_version: 1, status: "present", last_seen_at: now, ...timestamps };
const chat = { id: id(8), title: "资料问答", provider: "local", version: 1, ...timestamps };
const current = { id: id(2), user_id: id(3), current: true, created_at: now, last_seen_at: now,
  idle_expires_at: "2026-11-30T00:00:00Z", absolute_expires_at: "2026-12-19T00:00:00Z" };
const messages = Array.from({ length: 8 }, (_, i) => ({
  id: id(60 + i), session_id: chat.id, role: i % 2 === 0 ? "user" : "assistant", run_id: id(80 + Math.floor(i / 2)),
  body: i % 2 === 0 ? `调研材料第 ${i + 1} 条记录了什么？` : `这是第 ${i + 1} 条材料的摘要。资料包括引用、数据和复核信息。`,
  incomplete: false, created_at: now,
}));
const target = messages.at(-1)!;
const fixtureFileText = "合成文本文件用于核对鉴权预览界面。";

async function fixtures(page: Page, authenticated = true) {
  const fileItems = Array.from({ length: 20 }, (_, i) => ({ ...file, id: id(200 + i), name: `资料 ${i + 1}.txt` }));
  let restored = false;
  let activeSession = authenticated;
  let currentTask = task;
  let failTrashRefresh = false;
  await page.route("**/api/v1/**", (route) => {
    const { pathname, searchParams } = new URL(route.request().url());
    const json = (data: unknown) => route.fulfill({ json: data });
    switch (pathname.replace("/api/v1", "")) {
      case "/auth/session": return activeSession ? json({ session: current, csrf_token: "synthetic-redesign-fixture" }) :
        route.fulfill({ status: 401, json: { code: "unauthorized", message: "未登录" } });
      case "/auth/passkeys": return json(list([{ id: id(9), name: "验收凭据", created_at: now, last_used_at: now }]));
      case "/auth/sessions": return json(list([current]));
      case `/auth/sessions/${current.id}`:
        expect(route.request().method()).toBe("DELETE");
        activeSession = false;
        return route.fulfill({ status: 204 });
      case "/dashboard": return json({
        counts: { notes: 4, tasks_open: 7, tasks_due_today: 2, projects_active: 3, files: 21 },
        tasks_by_status: { todo: 3, in_progress: 4, done: 5, cancelled: 1 },
        recent_notes: [note], recent_tasks: [], due_tasks: [task, {
          ...task, id: id(14), title: "补全引用", due: { ...task.due, date: "2020-11-18" },
        }], job_failures: 1, indexing_enabled: false,
      });
      case "/notes": return json(list([note, { ...note, id: id(15), title: "后续问题", parent_id: note.id }]));
      case `/notes/${note.id}`:
        if (route.request().method() === "PATCH") return route.fulfill({
          status: 409, json: { code: "version_conflict", message: "版本冲突", current_version: 3 },
        });
        return json(note);
      case `/notes/${note.id}/snapshots`:
        if (route.request().method() === "POST") return json({ ...note, note_id: note.id, snapshot_id: id(51), reason: "flush" });
        return json(list([{
          ...note, note_id: note.id, snapshot_id: id(50), title: "历史调研手记", content_version: 1, reason: "flush",
        }]));
      case "/projects": return json(list([project]));
      case `/projects/${project.id}`: return json(project);
      case "/tasks":
        if (searchParams.has("parent_id")) return json(list([]));
        if (searchParams.get("limit") === "50" && searchParams.get("project_id") === project.id && !searchParams.has("status")) {
          return searchParams.has("cursor") ? json(list([currentTask])) : json(list([{ ...task, id: id(16), title: "第一页事项" }], "tasks-page-2"));
        }
        return json(list([currentTask]));
      case `/tasks/${task.id}`:
        if (route.request().method() === "PATCH") {
          const payload = route.request().postDataJSON();
          expect(payload).toMatchObject({ status: "in_progress", expected_version: currentTask.version });
          currentTask = { ...currentTask, status: payload.status, version: currentTask.version + 1 };
        }
        return json(currentTask);
      case `/tasks/${task.id}/activities`: return json(list([]));
      case "/files": return json(list([...fileItems, file]));
      case `/files/${file.id}`: return json(file);
      case `/files/${file.id}/content`: return route.fulfill({
        body: fixtureFileText, contentType: "text/plain; charset=utf-8", headers: { etag: '"1"' },
      });
      case "/jobs": return json(list([{
        id: id(12), kind: "file.extract", state: "failed", attempts: 1, max_attempts: 3, error: "unsupported_format",
        started_at: now, finished_at: now, ...timestamps,
      }]));
      case "/search": return json(list([{
        id: target.id, session_id: chat.id, entity_type: "chat", title: "会话中的调研材料",
        snippet: "调研资料包括引用、数据和复核信息。", path: null, score: null, updated_at: now,
      }]));
      case "/chat/sessions": return json(list([chat]));
      case `/chat/sessions/${chat.id}`: return json({ ...chat, messages });
      case "/trash": {
        if (failTrashRefresh) {
          return route.fulfill({ status: 503, json: { code: "dependency_unavailable", message: "刷新暂时失败" } });
        }
        return json(list([
          ...(!restored ? [{ id: id(10), title: "旧版手记", entity_type: "note", parent_id: id(11), version: 1,
            deleted_at: now, can_restore: false, reason: "原父笔记已删除，可恢复到根目录。" }] : []),
          { id: id(13), title: "待查证材料", entity_type: "task", parent_id: null, version: 1,
            deleted_at: now, can_restore: false, reason: "父事项已删除。" },
        ]));
      }
      case `/notes/${id(10)}/restore`: {
        if (route.request().method() !== "POST" || route.request().postDataJSON().restore_parent?.parent_id !== null) {
          return route.fulfill({ status: 422, json: { code: "invalid_request", message: "请选择根目录" } });
        }
        restored = true;
        failTrashRefresh = true;
        return json({ ...note, id: id(10) });
      }
      case "/tags":
      case "/links":
      case `/entities/note/${note.id}/tags`:
      case `/entities/note/${id(15)}/tags`:
      case `/entities/project/${project.id}/tags`:
        return json(list([]));
      default: throw new Error(`Unexpected fixture request: ${route.request().method()} ${pathname}`);
    }
  });
}

type RouteSample = {
  name: string;
  path: string;
  ready: (page: Page) => Locator;
  section?: string;
};
const heading = (name: string | RegExp) => (page: Page) => page.getByRole("heading", { name, exact: true }).first();
const routeSamples: RouteSample[] = [
  { name: "login", path: "/login", ready: heading("登录") },
  { name: "bootstrap", path: "/bootstrap", ready: heading("首次注册") },
  { name: "recovery", path: "/recovery", ready: heading("恢复账号") },
  { name: "dashboard", path: "/", ready: heading("工作概览"), section: "概览" },
  { name: "notes", path: "/notes", ready: page => page.getByRole("treeitem", { name: /调研手记/ }), section: "笔记" },
  { name: "note", path: `/notes/${note.id}`, ready: page => page.getByRole("textbox", { name: "笔记标题" }), section: "笔记" },
  { name: "projects", path: "/projects", ready: heading("项目"), section: "项目" },
  { name: "project", path: `/projects/${project.id}`, ready: heading(project.title), section: "项目" },
  { name: "tasks", path: "/tasks", ready: heading("事项"), section: "事项" },
  { name: "task", path: `/tasks/${task.id}`, ready: heading(/^校对调研报告/), section: "事项" },
  { name: "files", path: "/files", ready: heading("文件"), section: "文件" },
  { name: "file", path: `/files/${file.id}`, ready: heading(file.name), section: "文件" },
  { name: "jobs", path: "/files/jobs", ready: heading("索引任务"), section: "文件" },
  { name: "search", path: "/search?q=调研", ready: page => page.getByRole("link", { name: "会话中的调研材料" }), section: "搜索" },
  { name: "chat", path: "/chat", ready: page => page.getByRole("link", { name: chat.title }), section: "问答" },
  { name: "thread", path: `/chat/${chat.id}`, ready: heading(chat.title), section: "问答" },
  { name: "trash", path: "/trash", ready: heading("回收站"), section: "回收站" },
  { name: "security", path: "/settings/security", ready: heading("凭据与会话"), section: "凭据与会话" },
  { name: "404", path: "/missing-route", ready: heading("页面不存在") },
];

const baseline = process.env.KNOTED_REDESIGN_BASELINE === "1";
const capture = process.env.KNOTED_REDESIGN_CAPTURE;

for (const width of [1440, 768, 390, 320]) {
  for (const sample of routeSamples) {
    test(`矩阵 ${width}px ${sample.name}`, async ({ page }) => {
      test.skip(baseline && width !== 1440 && width !== 390, "基线仅记录桌面与手机对照");
      await page.setViewportSize({ width, height: width > 768 ? 900 : 844 });
      await fixtures(page, !(["login", "bootstrap", "recovery"].includes(sample.name)));
      const errors: Error[] = [];
      page.on("pageerror", error => errors.push(error));
      await page.goto(`http://127.0.0.1:4178${sample.path}`);
      await expect(sample.ready(page)).toBeVisible();
      if (sample.section) {
        await expect(page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: sample.section }))
          .toHaveAttribute("aria-current", "page");
      }
      if (!baseline) {
        await expect(sample.ready(page)).toBeInViewport();
        expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);
        expect(errors).toEqual([]);
      }
      if (capture && (width === 1440 || width === 390)) {
        await page.evaluate(() => document.fonts.ready.then(() => {}));
        await page.screenshot({ path: `${capture}/${width}-${sample.name}.png`, animations: "disabled" });
      }
    });
  }
}

test("窄屏文件列表打开详情，预览源码并关闭回焦，再返回列表", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/files");
  const fileRow = page.getByRole("button", { name: `打开文件 ${file.name}` });
  await fileRow.click();
  await expect(page.getByRole("heading", { name: file.name, level: 1 })).toBeInViewport();
  const preview = page.getByRole("button", { name: "预览文件" });
  await preview.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(fixtureFileText)).toBeVisible();
  await expect(dialog.getByRole("button", { name: "下载原文件" })).toBeInViewport();
  if (capture) await page.screenshot({ path: `${capture}/390-file-preview.png`, animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(preview).toBeFocused();
  await page.getByRole("link", { name: "返回文件列表" }).click();
  await expect(fileRow).toBeFocused();
  await expect(fileRow).toBeInViewport();
});

test("回收站确认关闭回焦，恢复成功但刷新失败时已恢复行移除", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/trash");
  const trigger = page.getByRole("button", { name: "恢复 旧版手记" });
  await trigger.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByText(/恢复到根目录/)).toBeVisible();
  if (capture) await page.screenshot({ path: `${capture}/390-trash-restore.png`, animations: "disabled" });
  await page.keyboard.press("Escape");
  await expect(trigger).toBeFocused();
  await page.keyboard.press("Enter");
  await dialog.getByRole("button", { name: "确认恢复" }).click();
  await expect(page.getByRole("heading", { name: "回收站", level: 1 })).toBeFocused();
  await expect(page.getByRole("status").getByText("已恢复 旧版手记")).toBeVisible();
  await expect(page.getByRole("alert").getByText("列表刷新失败")).toBeVisible();
  await expect(trigger).toHaveCount(0);
});

test("概览先读上海到期状态，打开事项后返回默认清单", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await page.clock.setFixedTime(new Date("2026-11-19T06:00:00Z"));
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/");
  const due = page.getByRole("list", { name: "到期事项" });
  const first = due.getByRole("listitem").filter({ hasText: task.title });
  await expect(first).toContainText("今日到期");
  await expect(due.getByRole("listitem").filter({ hasText: "补全引用" })).toContainText("已逾期");
  await first.getByRole("button", { name: `打开事项 ${task.title}` }).click();
  await expect(page.getByRole("heading", { name: task.title, level: 1 })).toBeInViewport();
  await expect(page.getByRole("heading", { name: "跟进时间线" })).toBeVisible();
  await page.getByRole("link", { name: "返回事项" }).click();
  await expect(page.getByRole("radio", { name: "列表视图" })).toBeChecked();
});

test("从筛选后的看板第二页查看事项，返回后使用键盘移列", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 1440, height: 900 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/tasks");
  await page.getByRole("radio", { name: "看板视图" }).click();
  await page.getByRole("combobox", { name: "项目筛选" }).click();
  await page.getByRole("option", { name: project.title }).click();
  await page.getByRole("button", { name: "下一页事项" }).click();
  await page.getByRole("link", { name: task.title }).click();
  await page.getByRole("link", { name: "返回事项" }).click();
  await expect(page.getByRole("radio", { name: "看板视图" })).toBeChecked();
  await expect(page.getByRole("combobox", { name: "项目筛选" })).toHaveText(project.title);
  await expect(page.getByRole("button", { name: "上一页事项" })).toBeEnabled();
  const card = page.getByRole("listitem").filter({ has: page.getByRole("link", { name: task.title }) });
  await expect(card).toHaveAttribute("aria-disabled", "false");
  await card.focus();
  await page.keyboard.press("Alt+ArrowRight");
  const moved = page.getByRole("list", { name: "进行中事项" }).getByRole("listitem");
  await expect(moved).toContainText(task.title);
  await expect(moved).toBeFocused();
  if (capture) await page.screenshot({ path: `${capture}/1440-board.png`, animations: "disabled" });
});

test("笔记历史立即可读，保存冲突后离开及退出均保留草稿", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);
  const editor = page.getByRole("textbox", { name: "笔记标题" });
  const history = page.getByRole("button", { name: "历史版本" });
  await history.click();
  await page.getByRole("button", { name: /预览 历史调研手记/ }).click();
  await expect(page.getByRole("heading", { name: "历史调研手记" })).toBeInViewport();
  const preview = page.getByRole("region", { name: "历史预览" });
  await expect(preview).toContainText("调研记录第 1 段");
  await expect(preview.locator('[contenteditable="true"]')).toHaveCount(0);
  if (capture) await page.screenshot({ path: `${capture}/390-note-history.png`, animations: "disabled" });
  await page.getByRole("button", { name: "关闭历史" }).click();
  await expect(history).toBeFocused();
  await editor.fill("当前草稿不能丢失");
  await page.getByRole("button", { name: "退出登录" }).click();
  await expect(editor).toHaveValue("当前草稿不能丢失");
  await expect(page.getByRole("button", { name: "版本与冲突处理" })).toBeVisible();
  await page.getByRole("button", { name: "目录", exact: true }).click();
  const directory = page.getByRole("dialog", { name: "笔记目录" });
  await expect(directory.getByRole("tree")).toBeVisible();
  if (capture) await page.screenshot({ path: `${capture}/390-note-directory.png`, animations: "disabled" });
  await directory.getByRole("button", { name: "关闭" }).click();
  await page.getByRole("navigation", { name: "主导航" }).getByRole("button", { name: "项目" }).click();
  await expect(page).toHaveURL(new RegExp(`/notes/${note.id}$`));
  await expect(editor).toHaveValue("当前草稿不能丢失");
});

test("安全页撤销当前会话，旧保护页重新打开仍需登录", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 390, height: 844 });
  await fixtures(page);
  await page.goto("http://127.0.0.1:4178/settings/security");
  await page.getByRole("button", { name: "撤销当前会话" }).click();
  await page.getByRole("alertdialog").getByRole("button", { name: "确认撤销" }).click();
  await expect(page.getByRole("heading", { name: "登录" })).toBeVisible();
  await page.goto("http://127.0.0.1:4178/settings/security");
  await expect(page.getByRole("heading", { name: "登录" })).toBeVisible();
});

test("200% 放大下正文仍可在当前工作区内阅读", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await page.setViewportSize({ width: 768, height: 900 });
  await fixtures(page);
  await page.goto(`http://127.0.0.1:4178/files/${file.id}`);
  await expect(page.getByRole("heading", { name: file.name, level: 1 })).toBeVisible();
  await page.evaluate(() => { document.documentElement.style.zoom = "2"; });
  await expect(page.getByRole("button", { name: "预览文件" })).toBeInViewport();
  expect(await page.getByRole("main").evaluate(main => main.scrollWidth - main.clientWidth)).toBeLessThanOrEqual(1);
});

test("静态图片在 Chromium 中实际解码，关闭后撤销对象 URL", async ({ page }) => {
  test.skip(baseline, "改版后的预览回归");
  const body = await readFile(resolve(__dirname, "../../apps/web/tests/fixtures/t50/chinese.png"));
  await fixtures(page);
  await page.route(`**/api/v1/files/${file.id}`, route => route.fulfill({
    json: { ...file, name: "调研插图.png", mime: "image/png", size: body.length },
  }));
  await page.route(`**/api/v1/files/${file.id}/content`, route => route.fulfill({
    body, contentType: "image/png", headers: { etag: '"1"' },
  }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`http://127.0.0.1:4178/files/${file.id}`);
  const preview = page.getByRole("button", { name: "预览文件" });
  await preview.click();
  const image = page.getByRole("img", { name: "调研插图.png" });
  await expect.poll(() => image.evaluate((element: HTMLImageElement) => element.naturalWidth)).toBeGreaterThan(0);
  const objectUrl = await image.getAttribute("src");
  expect(objectUrl).toMatch(/^blob:/);
  if (capture) await page.screenshot({ path: `${capture}/390-image-preview.png`, animations: "disabled" });
  await page.getByRole("button", { name: "关闭预览" }).click();
  await expect(preview).toBeFocused();
  expect(await page.evaluate(url => fetch(url!).then(() => true, () => false), objectUrl)).toBe(false);
});

test("笔记目录选中新笔记和切换到宽屏后焦点仍在可见界面", async ({ page }) => {
  test.skip(baseline, "改版后的布局回归");
  await fixtures(page);
  const child = { ...note, id: id(15), title: "后续问题", parent_id: note.id };
  await page.route(`**/api/v1/notes/${child.id}`, route => route.fulfill({ json: child }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`http://127.0.0.1:4178/notes/${note.id}`);
  const opener = page.getByRole("button", { name: "目录", exact: true });
  await opener.click();
  const dialog = page.getByRole("dialog", { name: "笔记目录" });
  const root = dialog.getByRole("treeitem", { name: /调研手记/ });
  await root.focus();
  await page.keyboard.press("ArrowRight");
  await dialog.getByRole("treeitem", { name: /后续问题/ }).click();
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue(child.title);
  await expect(dialog).toBeHidden();
  await expect(opener).toBeFocused();
  await opener.click();
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(dialog).toBeHidden();
  await expect(page.getByRole("heading", { name: "笔记目录" })).toBeFocused();
});
