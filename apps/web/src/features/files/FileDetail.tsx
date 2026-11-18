import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, ConfirmDialog, FormField, Input, MultiSelect, Spinner, Textarea } from "@bixlab/mtone";
import { ApiRequestError, type components } from "@knoted/api-client";
import { queryKeys, type QueryClientBundle } from "../../lib/query";
import { fileQuery, remember, type FileSource } from "./queries";
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
    const [targetId, setTargetId] = useState(""), [target, setTarget] = useState<FileSource>();
    const [tagPage, setTagPage] = useState<string | null>(null), [tagNames, setTagNames] = useState<Record<string, string>>({});
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
    async function download() {
        const owner = lifetime.current;
        if (!owner || busy)
            return;
        setBusy(true);
        setError("");
        try {
            const { data, response } = await bundle.api.raw.GET("/files/{fileId}/content", { params: { path: { fileId: source.id } }, parseAs: "blob", signal: owner.signal });
            if (!active(owner))
                return;
            if (!data)
                throw new Error("下载响应无效");
            if (response.headers.get("etag") !== `"${source.version}"`) {
                setNotice("文件版本已变化，请刷新后重试下载。");
                await bundle.queryClient.invalidateQueries({ queryKey: queryKeys.files.detail(source.id) });
                return;
            }
            const url = URL.createObjectURL(data);
            try {
                const link = document.createElement("a");
                link.href = url;
                link.download = Array.from(source.name, c => c.charCodeAt(0) < 32 || c.charCodeAt(0) === 127 || c === "/" || c === "\\" ? "_" : c).join("");
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
    return <section aria-label="文件详情" className="space-y-4"><h2>{source.name}</h2><dl><dt>来源 ID</dt><dd>{source.id}</dd><dt>相对路径</dt><dd>{source.relative_path}</dd><dt>大小</dt><dd>{source.size} 字节</dd><dt>SHA-256</dt><dd className="break-all">{source.sha256}</dd><dt>版本 / 内容版本</dt><dd>{source.version} / {source.content_version}</dd><dt>状态</dt><dd>{readOnly ? "已丢失" : "存在"}</dd><dt>最后出现</dt><dd>{source.last_seen_at ?? "未知"}</dd></dl>
 {readOnly && <Alert tone="info">来源已丢失，备注和标签只读，可手动重关联。</Alert>}
 <FormField label="文件备注"><Textarea value={note} disabled={locked} onChange={e => setNote(e.target.value)}/></FormField>
 <FormField label="文件标签"><MultiSelect value={tags} options={choices} disabled={locked} onValueChange={setTags}/></FormField>
 {options.error && <Alert tone="error">标签名称读取失败，已有标签 ID 保留。<Button onClick={() => void options.refetch()}>重试标签</Button></Alert>}
 <Button disabled={options.isFetching || !options.data?.next_cursor} onClick={() => setTagPage(options.data?.next_cursor ?? null)}>加载更多标签名称</Button>
 <Button disabled={locked || !dirty || conflict} onClick={() => void save()}>保存备注和标签</Button><Button disabled={busy || !dirty} onClick={() => { setBase(source); setNote(source.note ?? ""); setTags(source.tag_ids); setConflict(false); }}>丢弃草稿</Button>
 <Button disabled={busy || readOnly} onClick={() => void download()}>下载原文件</Button>
 {conflict && <Alert tone="warning">版本冲突，草稿已保留。</Alert>}{conflict && <><FormField label="文件草稿副本"><Textarea readOnly value={JSON.stringify(draft, null, 2)}/></FormField><Button disabled={busy} onClick={() => void refresh(false)}>刷新服务端信息，保留草稿</Button><ConfirmDialog title="用当前版本重试草稿？" description="保存会覆盖服务端的备注和标签。" confirmText="采用当前版本" trigger={<Button disabled={busy}>用当前版本重试草稿</Button>} onConfirm={() => refresh(true)}/></>}
 {notice && <p role="status">{notice}</p>}{error && <Alert tone="error">{error}</Alert>}
 <div className="space-y-2"><h3>手动重关联</h3><p>将当前来源的备注、标签和关联转移到明确选择的现存来源，旧引用快照保留。{dirty && "请先保存或丢弃草稿。"}</p><FormField label="重关联目标 UUID"><Input value={targetId} disabled={busy || dirty} onChange={e => { setTargetId(e.target.value); setTarget(undefined); }}/></FormField><Button disabled={busy || dirty} onClick={() => void inspectTarget()}>读取重关联目标</Button>
 {target && <ConfirmDialog title="确认重关联？" description={`从 ${source.id} (${source.relative_path}) 转移到 ${target.id} (${target.relative_path})。`} confirmText="确认重关联" trigger={<Button disabled={busy || dirty}>重关联到 {target.name}</Button>} onConfirm={relink}/>}</div>
 </section>;
}
