import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createApiClient } from "@knoted/api-client";
import { describe, expect, it } from "vitest";
import { NoteAssociations } from "../../src/features/notes/links/NoteAssociations";
import { UiProvider } from "../../src/lib/ui-provider";

const noteId = "00000000-0000-4000-8000-000000000001";
const tagId = "00000000-0000-4000-8000-000000000002";
const existingTagId = "00000000-0000-4000-8000-000000000004";
const targetId = "00000000-0000-4000-8000-000000000003";

function json(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), { status, headers: { "content-type": "application/json" } });
}

function harness(fail = false, gate?: Promise<void>, initialWorkspaceTags: { id: string; name: string; version: number }[] = []) {
  const requests: Request[] = [];
  let tags: { id: string; name: string; version: number }[] = [];
  const allTags = [...initialWorkspaceTags];
  let links: unknown[] = [];
  const api = createApiClient({
    baseUrl: "http://localhost/api/v1",
    fetch: async (input) => {
      const request = input as Request;
      requests.push(request.clone());
      const path = new URL(request.url).pathname;
      if (request.method === "GET") {
        await gate;
        if (path === "/api/v1/tags") return json({ items: allTags, next_cursor: null });
        return json({ items: path.endsWith("/links") ? links : tags, next_cursor: null });
      }
      if (fail) return json({ code: "dependency_unavailable", message: "服务暂不可用", request_id: noteId }, 503);
      if (path === "/api/v1/tags") {
        const created = { id: tagId, name: "研究", version: 1, created_at: "2026-11-17T00:00:00Z", updated_at: "2026-11-17T00:00:00Z" };
        allTags.push({ id: created.id, name: created.name, version: 1 });
        return json(created, 201);
      }
      if (path.includes(`/entities/note/${noteId}/tags/`)) {
        const id = path.split("/").at(-1)!;
        if (request.method === "PUT") {
          const found = allTags.find(item => item.id === id) ?? { id, name: "研究", version: 1 };
          if (!tags.some(item => item.id === id)) tags = [...tags, found];
        } else {
          tags = tags.filter(item => item.id !== id);
        }
        return new Response(null, { status: 204 });
      }
      if (path === "/api/v1/links") {
        const body = await request.json();
        const link = { id: "link-1", ...body, version: 1, created_at: "2026-11-17T00:00:00Z" };
        links = [link];
        return json(link, 201);
      }
      if (path.endsWith("/links/link-1")) {
        links = [];
        return new Response(null, { status: 204 });
      }
      throw new Error(`${request.method} ${path}`);
    },
  });
  render(
    <UiProvider>
      <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
        <NoteAssociations noteId={noteId} api={api} />
      </QueryClientProvider>
    </UiProvider>,
  );
  return requests;
}

async function openAssociations() {
  fireEvent.click(screen.getByRole("button", { name: "标签与关联" }));
  return screen.findByRole("dialog", { name: "标签与关联" });
}

describe("T32 note associations", () => {
  it("creates, selects existing, and removes tags via Select tags mode and manages stable identity links", async () => {
    const requests = harness(false, undefined, [{ id: existingTagId, name: "架构", version: 1 }]);
    const drawer = await openAssociations();
    const tagInput = within(drawer).getByRole("combobox", { name: "笔记标签" });
    await waitFor(() => expect(tagInput).toBeEnabled());

    // Select an existing workspace tag without creating a duplicate via POST /tags
    fireEvent.click(within(drawer).getByRole("button", { name: "展开选项" }));
    fireEvent.click(await screen.findByRole("option", { name: "架构" }));
    expect(await within(drawer).findByRole("button", { name: "移除 架构" })).toBeInTheDocument();
    expect(requests.filter(r => r.method === "POST" && new URL(r.url).pathname === "/api/v1/tags")).toHaveLength(0);

    // Create and assign a brand-new tag via Select tags mode
    if (tagInput.getAttribute("aria-expanded") !== "true") {
      fireEvent.click(within(drawer).getByRole("button", { name: "展开选项" }));
    }
    fireEvent.change(tagInput, { target: { value: "研究" } });
    fireEvent.click(await screen.findByRole("option", { name: "创建“研究”" }));
    expect(await within(drawer).findByRole("button", { name: "移除 研究" })).toBeInTheDocument();
    expect(requests.filter(r => r.method === "POST" && new URL(r.url).pathname === "/api/v1/tags")).toHaveLength(1);

    // Remove the created tag via its chip remove button
    fireEvent.click(within(drawer).getByRole("button", { name: "移除 研究" }));
    await waitFor(() => expect(within(drawer).queryByRole("button", { name: "移除 研究" })).not.toBeInTheDocument());
    expect(within(drawer).getByRole("button", { name: "移除 架构" })).toBeInTheDocument();

    fireEvent.click(within(drawer).getByRole("button", { name: "新建关联" }));
    fireEvent.change(screen.getByLabelText("关联目标 ID"), { target: { value: targetId } });
    fireEvent.click(screen.getByRole("button", { name: "添加关联" }));
    expect(await screen.findByText(targetId)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: `移除关联 ${targetId}` }));
    await waitFor(() => expect(screen.queryByText(targetId)).not.toBeInTheDocument());
    const created = requests.find(r => r.method === "POST" && new URL(r.url).pathname.endsWith("/links"));
    expect(await created?.json()).toEqual({ source_type: "note", source_id: noteId, target_type: "note", target_id: targetId });
    expect(requests.some(r => r.method === "DELETE" && r.url.endsWith("/links/link-1?expected_version=1"))).toBe(true);
  });

  it("disables mutations while loading and preserves the input on server error", async () => {
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    harness(true, gate);
    const drawer = await openAssociations();
    const tagInput = within(drawer).getByRole("combobox", { name: "笔记标签" });
    expect(tagInput).toBeDisabled();
    fireEvent.click(within(drawer).getByRole("button", { name: "新建关联" }));
    expect(screen.getByRole("button", { name: "添加关联" })).toBeDisabled();
    release();
    await waitFor(() => expect(tagInput).toBeEnabled());
    await waitFor(() => expect(screen.getByRole("button", { name: "添加关联" })).toBeEnabled());

    // Tag creation failure surfaces server error without adding a phantom chip
    fireEvent.click(within(drawer).getByRole("button", { name: "展开选项" }));
    fireEvent.change(tagInput, { target: { value: "失败标签" } });
    fireEvent.click(await screen.findByRole("option", { name: "创建“失败标签”" }));
    expect(await within(drawer).findByRole("alert")).toHaveTextContent("服务暂不可用");
    expect(within(drawer).queryByRole("button", { name: "移除 失败标签" })).not.toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("关联目标 ID"), { target: { value: targetId } });
    fireEvent.click(screen.getByRole("button", { name: "添加关联" }));
    await waitFor(() => expect(within(drawer).getAllByRole("alert")).toHaveLength(2));
    expect(screen.getByLabelText("关联目标 ID")).toHaveValue(targetId);
  });

  it("retains pending tag assignment retry across unrelated tag mutations and invalidates tag options on retry", async () => {
    const requests: Request[] = [];
    let tags: { id: string; name: string; version: number }[] = [{ id: existingTagId, name: "架构", version: 1 }];
    const allTags = [{ id: existingTagId, name: "架构", version: 1 }];
    let failNewTagAssign = true;
    const api = createApiClient({
      baseUrl: "http://localhost/api/v1",
      fetch: async (input) => {
        const request = input as Request;
        requests.push(request.clone());
        const path = new URL(request.url).pathname;
        if (request.method === "GET") {
          if (path === "/api/v1/tags") return json({ items: allTags, next_cursor: null });
          return json({ items: path.endsWith("/links") ? [] : tags, next_cursor: null });
        }
        if (request.method === "POST" && path === "/api/v1/tags") {
          const created = { id: tagId, name: "研究", version: 1, created_at: "2026-11-17T00:00:00Z", updated_at: "2026-11-17T00:00:00Z" };
          allTags.push({ id: created.id, name: created.name, version: 1 });
          return json(created, 201);
        }
        if (path.endsWith(`/tags/${tagId}`) && request.method === "PUT") {
          if (failNewTagAssign) {
            return json({ code: "dependency_unavailable", message: "服务暂不可用", request_id: noteId }, 503);
          }
          tags = [...tags, { id: tagId, name: "研究", version: 1 }];
          return new Response(null, { status: 204 });
        }
        if (path.endsWith(`/tags/${existingTagId}`) && request.method === "DELETE") {
          tags = tags.filter(item => item.id !== existingTagId);
          return new Response(null, { status: 204 });
        }
        throw new Error(`${request.method} ${path}`);
      },
    });
    render(
      <UiProvider>
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
          <NoteAssociations noteId={noteId} api={api} />
        </QueryClientProvider>
      </UiProvider>,
    );
    const drawer = await openAssociations();
    const tagInput = within(drawer).getByRole("combobox", { name: "笔记标签" });
    await waitFor(() => expect(tagInput).toBeEnabled());

    // POST /tags succeeds while PUT fails with 503
    fireEvent.click(within(drawer).getByRole("button", { name: "展开选项" }));
    fireEvent.change(tagInput, { target: { value: "研究" } });
    fireEvent.click(await screen.findByRole("option", { name: "创建“研究”" }));
    expect(await within(drawer).findByRole("button", { name: "重试添加标签 研究" })).toBeInTheDocument();

    // Unrelated tag removal clears errors.tags but keeps pending assignment recovery available
    fireEvent.click(within(drawer).getByRole("button", { name: "移除 架构" }));
    await waitFor(() => expect(within(drawer).queryByRole("button", { name: "移除 架构" })).not.toBeInTheDocument());
    expect(within(drawer).getByRole("button", { name: "重试添加标签 研究" })).toBeEnabled();

    const optionsGetsBeforeRetry = requests.filter(r => r.method === "GET" && new URL(r.url).pathname === "/api/v1/tags").length;
    failNewTagAssign = false;
    fireEvent.click(within(drawer).getByRole("button", { name: "重试添加标签 研究" }));
    expect(await within(drawer).findByRole("button", { name: "移除 研究" })).toBeInTheDocument();
    expect(within(drawer).queryByRole("button", { name: "重试添加标签 研究" })).not.toBeInTheDocument();
    expect(requests.filter(r => r.method === "POST" && new URL(r.url).pathname === "/api/v1/tags")).toHaveLength(1);
    await waitFor(() => {
      const optionsGetsAfterRetry = requests.filter(r => r.method === "GET" && new URL(r.url).pathname === "/api/v1/tags").length;
      expect(optionsGetsAfterRetry).toBeGreaterThan(optionsGetsBeforeRetry);
    });
  });
});
