import { expect, test, type Page } from "@playwright/test";
import { execFile } from "node:child_process";
import { createHash, X509Certificate } from "node:crypto";
import { chmod, readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

const execute = promisify(execFile);
const origin = "https://t71.knoted.test:17820";
const samplePath = "agent-fixtures/t71/observations.txt";
const firstContent = "观测记录：紫檀标本的识别词为 violetamber。\n";
const updatedContent = "观察记录已更新：识别词为 glaciercobalt。\n";

async function api<T>(page: Page, method: string, route: string, body?: object) {
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
    return { status: response.status, data: await response.json() as T };
  }, { method, route, body });
}

async function capturedFile(page: Page, content: string) {
  const sha256 = createHash("sha256").update(content).digest("hex");
  let id: string | undefined;
  await expect.poll(async () => {
    const result = await api<{ items: { id: string; relative_path: string; sha256: string; status: string }[] }>(
      page, "GET", "/files",
    );
    expect(result.status).toBe(200);
    id = result.data.items.find(file =>
      file.relative_path === samplePath && file.status === "present" && file.sha256 === sha256,
    )?.id;
    return id ?? "";
  }, { timeout: 90_000 }).toMatch(/^[a-f\d-]{36}$/);
  if (!id) throw Error("The scan did not publish the expected file");
  return id;
}

async function originalBytes(page: Page, id: string) {
  return page.evaluate(async id => {
    const response = await fetch(`/api/v1/files/${id}/content`);
    return { status: response.status, bytes: Array.from(new Uint8Array(await response.arrayBuffer())) };
  }, id);
}

async function search(page: Page, term: string) {
  const result = await api<{ items: { id: string; entity_type: string }[] }>(
    page, "GET", "/search?q=" + encodeURIComponent(term),
  );
  expect(result.status).toBe(200);
  return result.data.items.map(({ id, entity_type }) => `${entity_type}:${id}`).sort();
}

test("T71 自动扫描、原件预览及笔记项目事项搜索", async ({ playwright }) => {
  test.setTimeout(300_000);
  const run = process.env.T71_RUN;
  const artifact = process.env.T71_ARTIFACT;
  if (!run || path.dirname(run) !== "/data/knoted_dev/t71" ||
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
    const account = page.getByRole("button", { name: "账号菜单" });
    await account.click();
    await page.getByRole("menuitem", { name: "退出登录" }).click();
    await page.getByRole("button", { name: "使用 Passkey 登录", exact: true }).click();
    await expect(account).toBeVisible();

    await test.step("自动发现并预览文件原件", async () => {
      const id = await capturedFile(page, firstContent);
      expect(await originalBytes(page, id)).toEqual({ status: 200, bytes: [...Buffer.from(firstContent)] });
      await page.goto(`${origin}/files/${id}`);
      await page.getByRole("button", { name: "预览文件" }).click();
      await expect(page.getByText(firstContent.trim(), { exact: true })).toBeVisible();
    });

    await test.step("自动发现只读源中的更新并保留原始字节", async () => {
      const sample = path.join(run, "files", samplePath);
      await chmod(sample, 0o644);
      try {
        await writeFile(sample, updatedContent);
      } finally {
        await chmod(sample, 0o444);
      }
      const id = await capturedFile(page, updatedContent);
      expect(await originalBytes(page, id)).toEqual({ status: 200, bytes: [...Buffer.from(updatedContent)] });
      expect(await search(page, "violetamber")).toEqual([]);
      expect(await search(page, "glaciercobalt")).toEqual([]);
    });

    await test.step("全文搜索返回笔记、项目和事项", async () => {
      const term = "saffroncardinal";
      const expected: string[] = [];
      for (const [entityType, route, body] of [
        ["note", "/notes", { title: `笔记 ${term}`, document: { blocks: [] } }],
        ["project", "/projects", { title: `项目 ${term}` }],
        ["task", "/tasks", { title: `事项 ${term}` }],
      ] as const) {
        const result = await api<{ id: string }>(page, "POST", route, body);
        expect(result.status).toBe(201);
        expected.push(`${entityType}:${result.data.id}`);
      }
      await expect.poll(() => search(page, term), { timeout: 90_000 }).toEqual(expected.sort());
    });
  } finally {
    await browser.close();
  }
});
