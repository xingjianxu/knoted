import { expect, test, type Page } from "@playwright/test";
import { execFile } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { access, chmod, readFile, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

const execute = promisify(execFile);
const origin = "https://t71.knoted.test:17820";
const oldTerm = "violetamber";
const newTerm = "glaciercobalt";

async function queryPrivateDatabase(run: string, sql: string) {
  const result = await execute("psql", [
    "-XAt", "-h", path.join(run, "socket"), "-p", "17828", "-d", "t71", "-c", sql,
  ]);
  return result.stdout.trim();
}

async function api(page: Page, method: string, route: string, body?: object) {
  return page.evaluate(async ({ method, route, body }) => {
    const csrf = document.cookie.split("; ").find(item => item.startsWith("knoted_csrf="))?.slice(12);
    const response = await fetch("/api/v1" + route, {
      method,
      headers: {
        "Content-Type": "application/json",
        "Idempotency-Key": crypto.randomUUID(),
        ...(csrf ? { "X-CSRF-Token": csrf } : {}),
      },
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    return { status: response.status, data: await response.json() };
  }, { method, route, body });
}
async function search(page: Page, term: string): Promise<string[]> {
  const response = await api(page, "GET", "/search?q=" + term + "&type=file");
  expect(response.status).toBe(200);
  return response.data.items.map((item: { id: string }) => item.id);
}

test("t71_1 file citation, saved note and linked task; t71_2 stale source recovery and visible vector failure", async ({ playwright }) => {
  test.setTimeout(540_000);
  const run = process.env.T71_RUN;
  const artifact = process.env.T71_ARTIFACT;
  if (!run || path.dirname(run) !== "/data/knoted-dev/tasks/t71/tmp" ||
      !path.basename(run).startsWith("instance.") || !artifact) {
    throw Error("Run the test through tests/fixtures/t71/instance.sh with a fixed T71_ARTIFACT");
  }
  const certificate = new X509Certificate(await readFile(path.join(run, "tls.crt")));
  const spki = certificate.publicKey.export({ type: "spki", format: "der" });
  const browser = await playwright.chromium.launch({
    executablePath: process.env.T71_CHROMIUM,
    args: ["--no-proxy-server", "--host-resolver-rules=MAP t71.knoted.test 127.0.0.1",
      "--ignore-certificate-errors-spki-list=" + createHash("sha256").update(spki).digest("base64")],
  });
  try {
    const page = await browser.newPage();
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("WebAuthn.enable");
    await cdp.send("WebAuthn.addVirtualAuthenticator", { options: {
      protocol: "ctap2", transport: "internal", hasResidentKey: true,
      hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true,
    } });
    await page.goto(origin + "/bootstrap");
    const { stdout } = await execute(path.join(artifact, "bin/knoted-api"), [
      "admin", "token", "--purpose", "bootstrap", "--config", path.join(run, "app.toml"),
    ]);
    const token = stdout.match(/^token: (.+)$/m)?.[1];
    if (!token) throw Error("Private instance bootstrap token unavailable");
    await page.getByLabel("一次性令牌").fill(token);
    await page.getByLabel("凭据名称", { exact: true }).fill("T71 合成 Passkey");
    await page.getByRole("button", { name: "注册 Passkey", exact: true }).click();
    await expect(page.getByRole("button", { name: "退出登录" })).toBeVisible();
    await page.getByRole("button", { name: "退出登录" }).click();
    await page.getByRole("button", { name: "使用 Passkey 登录", exact: true }).click();
    await expect(page.getByRole("button", { name: "退出登录" })).toBeVisible();
    // The worker performs its initial scan on startup.
    await expect(async () => {
      expect(await search(page, oldTerm)).toHaveLength(1);
    }).toPass({ timeout: 120_000 });
    const [fileId] = await search(page, oldTerm);
    expect(fileId).toMatch(/^[a-f\d-]{36}$/);
    const sql = "SELECT count(*) FROM rag_vectors v JOIN rag_embedding_sets e ON e.id=v.embedding_set_id " +
      "JOIN rag_chunk_sets s ON s.id=e.chunk_set_id WHERE vector_dims(v.embedding)=1024 AND s.source_id='" + fileId + "'";
    await expect.poll(async () => Number(await queryPrivateDatabase(run, sql)), { timeout: 240_000 }).toBeGreaterThan(0);

    await test.step("t71_1 ask about the file, save the cited answer, link the task", async () => {
      await page.goto(origin + "/chat");
      await page.getByRole("button", { name: "新建会话" }).click();
      const dialog = page.getByRole("dialog", { name: "新建会话" });
      await dialog.getByRole("textbox", { name: "会话名称" }).fill("T71 文件问答");
      await dialog.getByRole("button", { name: "创建会话" }).click();
      const thread = page.getByRole("region", { name: "会话详情" });
      await thread.getByRole("button", { name: "资料范围" }).click();
      await thread.getByRole("checkbox", { name: "文件" }).click();
      await thread.getByRole("textbox", { name: "问题" }).fill(oldTerm);
      await thread.getByRole("button", { name: "发送问题" }).click();
      const messages = page.getByRole("list", { name: "会话消息" });
      await expect(messages.getByText("观测资料记载了紫檀标本。", { exact: true })).toBeVisible({ timeout: 90_000 });
      await messages.getByRole("button", { name: "查看引用" }).click();
      const citations = messages.getByRole("list", { name: "引用来源" });
      await expect(citations.getByRole("link")).toHaveAttribute("href", "/files/" + fileId);
      await expect(citations).toContainText(oldTerm);
      await messages.getByRole("button", { name: "存为笔记" }).click();
      const save = page.getByRole("dialog", { name: "存为笔记" });
      await save.getByRole("textbox", { name: "标题" }).fill("T71 资料摘录");
      await save.getByRole("button", { name: "保存", exact: true }).click();
      await messages.getByRole("link", { name: "打开笔记" }).click();
      await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("T71 资料摘录");
      const note = await api(page, "GET", new URL(page.url()).pathname);
      expect(note.status).toBe(200);
      const text = JSON.stringify(note.data.document.blocks);
      expect(text).toContain("观测资料记载了紫檀标本。");
      expect(text).toContain(oldTerm);
      const task = await api(page, "POST", "/tasks", { title: "复核紫檀标本" });
      expect(task.status).toBe(201);
      const openAssociations = page.getByRole("button", { name: "标签与关联" });
      await openAssociations.click();
      const associations = page.getByRole("region", { name: "笔记标签与资料关联" });
      await associations.getByRole("button", { name: "新建关联" }).click();
      await associations.getByRole("combobox", { name: "关联目标类型" }).click();
      await page.getByRole("option", { name: "事项" }).click();
      await associations.getByLabel("关联目标 ID").fill(task.data.id);
      await associations.getByRole("button", { name: "添加关联" }).click();
      await expect(associations.getByRole("list", { name: "资料关联列表" })).toContainText(task.data.id);
      await page.reload();
      await openAssociations.click();
      await expect(associations.getByRole("list", { name: "资料关联列表" })).toContainText(task.data.id);
    });

    await test.step("t71_2 refuse publication, observe filtered stale hit, recover publication", async () => {
      const key = (await readFile(path.join(run, "meili.key"), "utf8")).trim();
      async function engineHasOld() {
        const res = await fetch("http://127.0.0.1:17824/indexes/t71_documents/search", {
          method: "POST", headers: { Authorization: "Bearer " + key, "Content-Type": "application/json" },
          body: JSON.stringify({ q: oldTerm, filter: 'source_type = "file"' }),
        });
        expect(res.status).toBe(200);
        return (await res.json()).hits.some((hit: {source_id: string}) => hit.source_id === fileId);
      }
      expect(await engineHasOld()).toBe(true);
      const sample = path.join(run, "files/agent-fixtures/t71/observations.txt");
      const updatedText = "更新记录：冰川钻芯的复验词为 " + newTerm + "，已复核。\n";
      expect(await readFile(sample, "utf8")).toBe("观测记录：紫檀标本的识别词为 violetamber。\n");
      const gate = path.join(run, "hold-writes");
      await writeFile(path.join(run, "fail-embedding-term"), newTerm);
      await writeFile(gate, "");
      try {
        // Only the fixture owner changes the synthetic original, as an external editor would.
        await chmod(sample, 0o600);
        try {
          await writeFile(sample, updatedText);
        } finally {
          await chmod(sample, 0o444);
        }
        await expect.poll(async () => {
          const result = await api(page, "POST", "/jobs/scan", { force: true });
          if (result.status === 409) expect(result.data.code).toBe("scan_pending");
          return result.status;
        }, { timeout: 45_000 }).toBe(201);
        await expect.poll(() => search(page, oldTerm), { timeout: 60_000 }).toHaveLength(0);
        await expect.poll(() => access(path.join(run, "write-refused")).then(() => true, () => false), { timeout: 60_000 }).toBe(true);
        expect(await engineHasOld()).toBe(true);
        expect(await search(page, newTerm)).toHaveLength(0);
      } finally {
        await unlink(gate);
      }
      await expect.poll(() => search(page, newTerm), { timeout: 120_000 }).toHaveLength(1);
      expect(await search(page, oldTerm)).toHaveLength(0);
      expect(await readFile(sample, "utf8")).toBe(updatedText);
    });

    await test.step("latest full text remains searchable while its vector job shows failure", async () => {
      const [currentId] = await search(page, newTerm);
      expect(currentId).toMatch(/^[a-f\d-]{36}$/);
      const witness = path.join(run, "invalid-result-id");
      await expect.poll(() => access(witness).then(() => true, () => false), { timeout: 180_000 }).toBe(true);
      const execution = await readFile(witness, "utf8");
      expect(execution).toMatch(/^[a-f\d-]{36}$/);
      const sql = `
        SELECT j.id FROM jobs j
        JOIN rag_chunk_sets s ON s.id::text = j.payload->>'chunk_set_id'
        JOIN rag_embedding_executions x ON x.job_id=j.id
        JOIN current_rag_sources live USING (source_type, source_id, source_version, extraction_id)
        WHERE s.source_type='file' AND s.source_id='${currentId}' AND j.kind='rag_vectors.embed'
          AND x.request->>'execution_id'='${execution}'
          AND j.last_error='embedding data or artifact is invalid' AND j.state='failed' AND NOT EXISTS (
            SELECT FROM rag_embedding_sets e WHERE e.chunk_set_id=s.id
          )`;
      const failedJob = () => queryPrivateDatabase(run, sql);
      await expect.poll(failedJob, { timeout: 30_000 }).toMatch(/^[a-f\d-]{36}$/);
      const id = await failedJob();
      await page.goto(origin + "/files/jobs");
      await page.getByRole("combobox", { name: "任务状态" }).click();
      const response = page.waitForResponse(res => {
        const url = new URL(res.url());
        return url.pathname === "/api/v1/jobs" && url.searchParams.get("state") === "failed";
      });
      await page.getByRole("option", { name: "失败", exact: true }).click();
      const failed = await response;
      expect(failed.status()).toBe(200);
      const vectors = (await failed.json()).items.filter((job: { kind: string }) => job.kind === "rag_vectors.embed");
      expect(vectors.map((job: { id: string }) => job.id)).toEqual([id]);
      const stage = page.getByRole("region", { name: "向量索引" });
      await expect(stage.getByText("嵌入与发布向量", { exact: true })).toBeVisible();
      await expect(stage.getByText("失败", { exact: true })).toBeVisible();
      await expect(stage.getByRole("button", { name: "重试任务" })).toBeVisible();
      expect(await search(page, newTerm)).toEqual([currentId]);
    });
  } finally {
    await browser.close();
  }
});
