import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, ConfirmDialog, Form, Input, Paragraph, PropertyItem, PropertyList, SectionGroup, Select, Spinner, Status } from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { fileQuery, remember, type FileSource } from "./queries";
import { FilePreview } from "./preview/FilePreview";
const message = (e: unknown) => e instanceof Error ? e.message : "操作失败，请重试";
const same = (a: string[], b: string[]) => [...a].sort().join() === [...b].sort().join();
export function FileDetail({ bundle, fileId, onSelectFile }: {
    bundle: QueryClientBundle;
    fileId: string;
    onSelectFile: (id: string) => void;
}) {
    const query = useQuery(fileQuery(bundle, fileId));
    if (!query.data)
        return query.isPending ? <Spinner label="正在读取文件详情"/> : <Alert tone="error">{query.error?.message}<Button onClick={() => void query.refetch()}>重试文件详情</Button></Alert>;
    return <FileEditor key={fileId} bundle={bundle} source={query.data} onSelectFile={onSelectFile}/>;
}
function FileEditor({ bundle, source, onSelectFile }: {
    bundle: QueryClientBundle;
    source: FileSource;
    onSelectFile: (id: string) => void;
}) {
    const [base, setBase] = useState(source), [note, setNote] = useState(source.note ?? ""), [tags, setTags] = useState(source.tag_ids);
    const [busy, setBusy] = useState(false), [error, setError] = useState(""), [notice, setNotice] = useState(""), [conflict, setConflict] = useState(false), [revoked, setRevoked] = useState(false);
    const [relinkOpen, setRelinkOpen] = useState(false);
    const [targetId, setTargetId] = useState(""), [target, setTarget] = useState<FileSource>();
    const [tagPage, setTagPage] = useState<string | null>(null), [tagNames, setTagNames] = useState<Record<string, string>>({});
    const [preview, setPreview] = useState<FileSource | null>(null);
    const previewEntry = useRef<HTMLButtonElement | null>(null);
    const lifetime = useRef<AbortController | null>(null);
    useEffect(() => { const owner = new AbortController(); lifetime.current = owner; return () => { owner.abort(); }; }, []);
    const noteChanged = note !== (base.note ?? "");
    const tagsChanged = !same(tags, base.tag_ids);
    const dirty = noteChanged || tagsChanged;
    if (!dirty && !busy && source !== base && source.version >= base.version) {
        setBase(source);
        setNote(source.note ?? "");
        setTags(source.tag_ids);
    }
    const options = useQuery({ queryKey: ["tag-options", tagPage], queryFn: async ({ signal }) => { const { data } = await bundle.api.raw.GET("/tags", { params: { query: { cursor: tagPage ?? undefined, limit: 50 } }, signal }); if (!data)
            throw new Error("标签响应无效"); return data; } });
    const [lastOptions, setLastOptions] = useState<typeof options.data>();
    if (options.data && options.data !== lastOptions) {
        setLastOptions(options.data);
        setTagNames(old => ({ ...old, ...Object.fromEntries(options.data!.items.map(tag => [tag.id, tag.name])) }));
    }
    const active = (owner: AbortController) => !owner.signal.aborted;
    function failure(e: unknown, owner: AbortController) { if (!active(owner))
        return; if (e instanceof ApiRequestError && e.status === 401) {
        owner.abort();
        setRevoked(true);
        setNote("");
        setTags([]);
        setTarget(undefined);
        return;
    } setError(message(e)); if (e instanceof ApiRequestError && e.status === 409)
        setConflict(true); }
    async function refreshList(owner: AbortController) { try {
        await bundle.queryClient.invalidateQueries({ queryKey: ["files", "list"] }, { throwOnError: true });
        if (active(owner))
            setNotice("保存成功。");
    }
    catch {
        if (active(owner))
            setNotice("保存成功，列表刷新失败。");
    } }
    async function save() {
        const owner = lifetime.current;
        if (!owner || busy || !dirty)
            return;
        setBusy(true);
        setError("");
        setNotice("");
        try {
            await bundle.queryClient.cancelQueries({ queryKey: queryKeys.files.detail(source.id) });
            if (!active(owner))
                return;
            // Omission preserves untouched fields, including soft-deleted tag
            // history and a stored empty-string note. Explicit rebase changes
            // base, so the retained draft is compared again before retrying.
            const body: components["schemas"]["FileSourceUpdate"] = {
                expected_version: base.version,
                ...(noteChanged ? { note: note === "" ? null : note } : {}),
                ...(tagsChanged ? { tag_ids: tags } : {}),
            };
            const { data } = await bundle.api.raw.PATCH("/files/{fileId}", { params: { path: { fileId: source.id } }, body, signal: owner.signal });
            if (!active(owner))
                return;
            if (!data || data.id !== source.id || data.version < base.version)
                throw new Error("保存响应无效，请刷新核对");
            remember(bundle, data);
            setBase(data);
            setNote(data.note ?? "");
            setTags(data.tag_ids);
            setConflict(false);
            setNotice("保存成功。");
            void refreshList(owner);
        }
        catch (e) {
            failure(e, owner);
        }
        finally {
            if (active(owner))
                setBusy(false);
        }
    }
    async function refresh(rebase: boolean) {
        const owner = lifetime.current;
        if (!owner || busy)
            return;
        setBusy(true);
        setError("");
        try {
            const latest = await bundle.queryClient.fetchQuery({ ...fileQuery(bundle, source.id), staleTime: 0 });
            if (!active(owner))
                return;
            if (rebase) {
                setBase(latest);
                setConflict(false);
                setNotice("已采用服务端版本，保存将覆盖服务端备注和标签。");
            }
            else
                setNotice("已刷新服务端信息，草稿和基础版本保持不变。");
        }
        catch (e) {
            failure(e, owner);
        }
        finally {
            if (active(owner))
                setBusy(false);
        }
    }
    async function download(selected: FileSource, reportToPreview = false) {
        const owner = lifetime.current;
        if (!owner || !active(owner) || busy || selected.id !== source.id || selected.status !== "present")
            return;
        setBusy(true);
        setError("");
        try {
            const { data, response } = await bundle.api.raw.GET("/files/{fileId}/content", { params: { path: { fileId: selected.id } }, parseAs: "blob", signal: owner.signal });
            if (!active(owner))
                return;
            if (!data)
                throw new Error("下载响应无效");
            if (response.headers.get("etag") !== `"${selected.version}"`) {
                setNotice("文件版本已变化，请刷新后重试下载。");
                await bundle.queryClient.invalidateQueries({ queryKey: queryKeys.files.detail(selected.id) });
                return "stale" as const;
            }
            const url = URL.createObjectURL(data);
            try {
                const link = document.createElement("a");
                link.href = url;
                link.download = Array.from(selected.name, c => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127 || c === "/" || c === "\\" ? "_" : c).join("");
                document.body.append(link);
                link.click();
                link.remove();
            }
            finally {
                URL.revokeObjectURL(url);
            }
        }
        catch (e) {
            failure(e, owner);
            if (reportToPreview)
                throw e;
        }
        finally {
            if (active(owner))
                setBusy(false);
        }
    }
    async function inspectTarget() {
        const owner = lifetime.current;
        if (!owner || busy)
            return;
        setBusy(true);
        setTarget(undefined);
        setError("");
        try {
            if (targetId.toLowerCase() === source.id || !/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(targetId))
                throw new Error("请输入不同来源的 UUID");
            const { data } = await bundle.api.raw.GET("/files/{fileId}", { params: { path: { fileId: targetId.toLowerCase() } }, signal: owner.signal });
            if (!active(owner))
                return;
            if (!data || data.id !== targetId.toLowerCase() || data.status !== "present")
                throw new Error("目标必须为存在的来源");
            setTarget(data);
        }
        catch (e) {
            failure(e, owner);
        }
        finally {
            if (active(owner))
                setBusy(false);
        }
    }
    async function relink() {
        const owner = lifetime.current;
        if (!owner || busy || dirty || !target)
            return;
        setBusy(true);
        setError("");
        try {
            const { data } = await bundle.api.raw.POST("/files/{fileId}/relink", { params: { path: { fileId: source.id } }, body: { expected_version: base.version, new_source_id: target.id }, signal: owner.signal });
            if (!active(owner))
                return;
            if (!data || data.id !== target.id)
                throw new Error("重关联响应无效");
            remember(bundle, data);
            void bundle.queryClient.invalidateQueries({ queryKey: queryKeys.files.all() });
            onSelectFile(data.id);
        }
        catch (e) {
            failure(e, owner);
        }
        finally {
            if (active(owner))
                setBusy(false);
        }
    }
    if (revoked)
        return <Alert tone="warning">会话已失效，请重新登录。</Alert>;
    const readOnly = source.status === "missing", locked = busy || readOnly;
    const choices = Array.from(new Set([...Object.keys(tagNames), ...tags])).map(id => ({ value: id, label: tagNames[id] ?? id }));
    const draft: components["schemas"]["FileSourceUpdate"] = { expected_version: base.version, note: note || null, tag_ids: tags };
    const relinkForm = (
      <div id="file-relink" hidden={!relinkOpen}>
          {relinkOpen && <div className="grid gap-2">
            <p>将备注、标签和关联转移到所选来源，原引用保留快照。{dirty && "请先保存或丢弃草稿。"}</p>
            <Form.Item label="重关联目标 UUID"><Input value={targetId} disabled={busy || dirty} onChange={e => {
              setTargetId(e.target.value); setTarget(undefined);
            }} /></Form.Item>
            <div className="flex flex-wrap gap-2">
              <Button disabled={busy || dirty} onClick={() => void inspectTarget()}>读取重关联目标</Button>
              {target && <ConfirmDialog title="确认重关联？"
                description={`从 ${source.id} (${source.relative_path}) 转移到 ${target.id} (${target.relative_path})。`}
                confirmText="确认重关联" trigger={<Button disabled={busy || dirty}>重关联到 {target.name}</Button>} onConfirm={relink} />}
            </div>
          </div>}
        </div>
    );
    return <section aria-labelledby="file-title" className="min-w-0 space-y-5">
      <header className="space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <h1 id="file-title">{source.name}</h1>
            <Status tone={readOnly ? "orange" : "green"}>{readOnly ? "已丢失" : "存在"}</Status>
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="solid" ref={previewEntry} disabled={busy || readOnly} onClick={event => {
              event.currentTarget.focus();
              setPreview(source);
            }}>预览文件</Button>
            <Button disabled={busy || readOnly} onClick={() => void download(source)}>下载原文件</Button>
          </div>
        </div>
        <Paragraph muted className="wrap-anywhere">{source.relative_path}</Paragraph>
      </header>
 <FilePreview api={bundle.api} file={preview} open={preview !== null} onDownload={selected => download(selected, true)} onOpenChange={open => {
        if (open) return;
        setPreview(null);
        const owner = lifetime.current;
        // The business opener can disappear on navigation/session loss. Never
        // move focus back into a different file or an expired session.
        queueMicrotask(() => {
            const entry = previewEntry.current;
            if (owner && active(owner) && entry?.isConnected && !entry.disabled) entry.focus();
        });
    }}/>

      {notice && <p role="status">{notice}</p>}
      {error && <Alert tone="error">{error}</Alert>}
      {readOnly && <div className="grid gap-2">
        <Alert tone="info" title="来源已丢失，备注和标签只读">
          <Button aria-expanded={relinkOpen} aria-controls="file-relink" onClick={() => setRelinkOpen(!relinkOpen)}>手动重关联</Button>
        </Alert>
        {relinkForm}
      </div>}

      <SectionGroup>
        <SectionGroup.Section title="备注与标签" headingLevel={2}>
          <div className="grid gap-3">
            <Form.Item label="文件备注"><Input.TextArea value={note} disabled={locked} onChange={e => setNote(e.target.value)} /></Form.Item>
            <Form.Item label="文件标签"><Select mode="multiple" value={tags} options={choices} disabled={locked} onValueChange={setTags} /></Form.Item>
            {options.error && <Alert tone="error">标签名称读取失败，已有标签 ID 保留。<Button onClick={() => void options.refetch()}>重试标签</Button></Alert>}
            {options.data?.next_cursor && <div><Button disabled={options.isFetching} onClick={() => setTagPage(options.data?.next_cursor ?? null)}>加载更多标签名称</Button></div>}
            {!readOnly && <div className="flex flex-wrap gap-2">
              <Button variant="solid" disabled={locked || !dirty || conflict} onClick={() => void save()}>保存备注和标签</Button>
              <Button disabled={busy || !dirty} onClick={() => {
                setBase(source); setNote(source.note ?? ""); setTags(source.tag_ids); setConflict(false);
              }}>丢弃草稿</Button>
            </div>}
            {conflict && <>
              <Alert tone="warning">版本冲突，草稿已保留。</Alert>
              <Form.Item label="文件草稿副本"><Input.TextArea readOnly value={JSON.stringify(draft, null, 2)} /></Form.Item>
              <div className="flex flex-wrap gap-2">
                <Button disabled={busy} onClick={() => void refresh(false)}>刷新服务端信息，保留草稿</Button>
                <ConfirmDialog title="用当前版本重试草稿？" description="保存会覆盖服务端的备注和标签。"
                  confirmText="采用当前版本" trigger={<Button disabled={busy}>用当前版本重试草稿</Button>} onConfirm={() => refresh(true)} />
              </div>
            </>}
          </div>
        </SectionGroup.Section>

        <SectionGroup.Section title="来源属性" headingLevel={2}>
          <div className="grid gap-2">
            <PropertyList>
              <PropertyItem label="来源 ID"><span className="wrap-anywhere">{source.id}</span></PropertyItem>
              <PropertyItem label="大小">{source.size} 字节</PropertyItem>
              <PropertyItem label="SHA-256"><span className="wrap-anywhere">{source.sha256}</span></PropertyItem>
              <PropertyItem label="版本 / 内容版本">{source.version} / {source.content_version}</PropertyItem>
              <PropertyItem label="最后出现">{source.last_seen_at ?? "未知"}</PropertyItem>
            </PropertyList>
            {!readOnly && <div><Button aria-expanded={relinkOpen} aria-controls="file-relink" onClick={() => setRelinkOpen(!relinkOpen)}>手动重关联</Button></div>}
            {!readOnly && relinkForm}
          </div>
        </SectionGroup.Section>
      </SectionGroup>
    </section>;
}
