import { useReducer } from "react";
import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Empty, List, ListItem, Spinner } from "@bixlab/mtone";
import type { QueryClientBundle } from "../../lib/query";
import { createCursorState, cursorReducer } from "../../lib/cursor-state";
import { filesQuery, type FileSource } from "./queries";
import { FileDetail } from "./FileDetail";
export function FileList({ items, onSelectFile }: {
    items: FileSource[];
    onSelectFile: (id: string) => void;
}) {
    return <><List aria-label="文件列表项">{items.map(file => <ListItem key={file.id} description={`${file.relative_path} · ${file.status === "present" ? "存在" : "已丢失"}`} actions={<Button aria-label={`打开文件 ${file.name}`} onClick={() => onSelectFile(file.id)}>打开</Button>}>{file.name}</ListItem>)}</List>{!items.length && <Empty title="暂无文件"/>}</>;
}
export function FilesPage({ bundle, fileId, onSelectFile }: {
    bundle: QueryClientBundle;
    fileId?: string;
    onSelectFile: (id: string | null) => void;
}) {
    const [page, dispatch] = useReducer(cursorReducer, undefined, () => createCursorState("files"));
    const list = useQuery(filesQuery(bundle, page.cursor));
    return <div className="space-y-6"><section aria-label="文件列表" className="space-y-3"><h2>文件</h2>
 {list.isPending && <Spinner label="正在读取文件列表"/>}{list.error && <Alert tone="error">{list.error.message}<Button onClick={() => void list.refetch()}>重试文件列表</Button></Alert>}
 {list.data && <FileList items={list.data.items} onSelectFile={onSelectFile}/>}
 <Button disabled={list.isFetching || !page.previous.length} onClick={() => dispatch({ type: "previous" })}>上一页文件</Button><Button disabled={list.isFetching || !list.data?.next_cursor} onClick={() => dispatch({ type: "next", nextCursor: list.data?.next_cursor ?? null })}>下一页文件</Button>
 </section>{fileId && <FileDetail key={fileId} bundle={bundle} fileId={fileId} onSelectFile={onSelectFile}/>}</div>;
}
