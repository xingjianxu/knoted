import { createAppRouter,safeReturnTo } from "../../src/lib/router";
import { createMemoryHistory } from "@tanstack/react-router";
import App from "../../src/App";
import { act,fireEvent,render,screen,waitFor } from "@testing-library/react";
import { QueryClient,QueryClientProvider } from "@tanstack/react-query";
import { createApiClient,type components } from "@knoted/api-client";
import { describe,it,expect,vi } from "vitest";
import { FilesPage } from "../../src/features/files/FilesPage";
import { UiProvider } from "../../src/lib/ui-provider";
import { queryKeys,type QueryClientBundle } from "../../src/lib/query";
const a="00000000-0000-4000-8000-000000000001",b="00000000-0000-4000-8000-000000000002";
type FileSource=components["schemas"]["FileSource"];
function file(id=a):FileSource{return {id,name:id===a?"alpha.txt":"beta.txt",relative_path:id===a?"folder/alpha.txt":"beta.txt",size:12,sha256:"a".repeat(64),version:1,content_version:7,status:"present",note:"original",tag_ids:[b],mime:null,created_at:"2026-11-18T00:00:00Z",updated_at:"2026-11-18T00:00:00Z",last_seen_at:null};}
const json=(data:unknown,status=200)=>new Response(JSON.stringify(data),{status,headers:{"content-type":"application/json"}});
function setup(){
 const rows=[file(),file(b)],patches:unknown[]=[];let failList=false;let hold:((r:Response)=>void)|undefined;let holdPatch=false;let contentHook:(()=>Response|Promise<Response>)|undefined;let detailHook:(()=>Response|Promise<Response>)|undefined;let listHook:(()=>Response|Promise<Response>)|undefined;
 const fetch:typeof globalThis.fetch=async input=>{const req=input as Request;const url=new URL(req.url);const path=url.pathname.replace("/api/v1","");
 if(path==="/auth/session")return json({session:{id:a,user_id:b,current:true},csrf_token:"test-csrf-value-long"});
 if(path==="/tags")return json({items:[],next_cursor:null});
 if(path==="/files"&&listHook)return listHook();
 if(path==="/files")return failList?json({code:"dependency_unavailable",message:"offline",request_id:a},503):json({items:rows,next_cursor:null});
 const row=rows.find(r=>r.id===path.split("/")[2]);if(!row)return json({},404);
 if(req.method==="PATCH"){const body=await req.json();patches.push(body);if(holdPatch)return new Promise<Response>(r=>{hold=r;});if(body.expected_version!==row.version)return json({code:"version_conflict",message:"conflict",current_version:row.version,request_id:a},409);Object.assign(row,body,{version:row.version+1});return json(row);}
 if(path.endsWith("/relink")){const body=await req.json();return json({...rows.find(r=>r.id===body.new_source_id)!,version:2});}
 if(path.endsWith("/content")&&contentHook)return contentHook();
 if(path.endsWith("/content"))return new Response("source bytes",{headers:{etag:`"${row.version}"`}});
 if(detailHook)return detailHook();
 return json(row);
 };
 const queryClient=new QueryClient({defaultOptions:{queries:{retry:false,staleTime:Infinity}}});const bundle={queryClient,api:createApiClient({baseUrl:"http://localhost/api/v1",fetch})};
 return {bundle,rows,patches,failList:()=>{failList=true},hold:()=>{holdPatch=true},resolve:(row:FileSource)=>hold?.(json(row)),content:(fn:()=>Response|Promise<Response>)=>{contentHook=fn},detail:(fn:()=>Response|Promise<Response>)=>{detailHook=fn},list:(fn:()=>Response|Promise<Response>)=>{listHook=fn}};
}
function view(bundle:QueryClientBundle,id=a){return <UiProvider><QueryClientProvider client={bundle.queryClient}><FilesPage bundle={bundle} fileId={id} onSelectFile={()=>{}} /></QueryClientProvider></UiProvider>;}
describe("T49 files",()=>{
 it("keeps dirty CAS and all tag IDs when a new server version arrives",async()=>{
 const h=setup();render(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:" local "}});
 h.rows[0]!.version=2;act(()=>{h.bundle.queryClient.setQueryData(queryKeys.files.detail(a),{...h.rows[0]!});});
 fireEvent.click(screen.getByRole("button",{name:"保存备注和标签"}));await screen.findByText("版本冲突，草稿已保留。");
 expect(h.patches).toEqual([{expected_version:1,note:" local ",tag_ids:[b]}]);expect(screen.getByLabelText("文件备注")).toHaveValue(" local ");
 });
 it("acknowledges save even when list refresh fails",async()=>{
 const h=setup();render(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"saved"}});h.failList();fireEvent.click(screen.getByRole("button",{name:"保存备注和标签"}));await screen.findByText("保存成功，列表刷新失败。");expect(screen.getByRole("button",{name:"保存备注和标签"})).toBeDisabled();
 fireEvent.change(screen.getByLabelText("文件备注"),{target:{value:"saved again"}});fireEvent.click(screen.getByRole("button",{name:"保存备注和标签"}));await waitFor(()=>expect(h.patches).toHaveLength(2));expect(h.patches[1]).toMatchObject({expected_version:2});
 });
 it("ignores a late save after A to B to A",async()=>{
 const h=setup();const rendered=render(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"old draft"}});h.hold();fireEvent.click(screen.getByRole("button",{name:"保存备注和标签"}));await waitFor(()=>expect(h.patches).toHaveLength(1));
 rendered.rerender(view(h.bundle,b));await screen.findByDisplayValue("original");rendered.rerender(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"new draft"}});
 await act(async()=>h.resolve({...file(),version:2,note:"old draft"}));expect(screen.getByLabelText("文件备注")).toHaveValue("new draft");
 });
 it("downloads with authenticated client and revokes its Blob URL",async()=>{
 const h=setup();const create=vi.fn(()=>"blob:test"),revoke=vi.fn();vi.stubGlobal("URL",Object.assign(URL,{createObjectURL:create,revokeObjectURL:revoke}));const click=vi.spyOn(HTMLAnchorElement.prototype,"click").mockImplementation(()=>{});
 render(view(h.bundle));await screen.findByLabelText("文件备注");fireEvent.click(screen.getByRole("button",{name:"下载原文件"}));await waitFor(()=>expect(create).toHaveBeenCalledOnce());expect(revoke).toHaveBeenCalledWith("blob:test");click.mockRestore();
 });
});

it("missing detail retains unknown tags and disables ordinary editing/download",async()=>{
 const h=setup();h.rows[0]!.status="missing";render(view(h.bundle));expect(await screen.findByLabelText("文件备注")).toBeDisabled();expect(screen.getByRole("button",{name:"下载原文件"})).toBeDisabled();expect(screen.getByText(b,{exact:true})).toBeInTheDocument();expect(screen.getByRole("button",{name:"读取重关联目标"})).toBeEnabled();
});
it("rejects mismatched download ETag without creating a URL",async()=>{
 const h=setup(),create=vi.fn();vi.stubGlobal("URL",Object.assign(URL,{createObjectURL:create,revokeObjectURL:vi.fn()}));h.content(()=>new Response("bytes",{headers:{etag:'"2"'}}));render(view(h.bundle));await screen.findByLabelText("文件备注");fireEvent.click(screen.getByRole("button",{name:"下载原文件"}));await screen.findByText("文件版本已变化，请刷新后重试下载。");expect(create).not.toHaveBeenCalled();
});
it("does not create a download from a late response after switching identity",async()=>{
 const h=setup();let resolve!:(r:Response)=>void;h.content(()=>new Promise<Response>(r=>{resolve=r}));const create=vi.fn();vi.stubGlobal("URL",Object.assign(URL,{createObjectURL:create,revokeObjectURL:vi.fn()}));const rendered=render(view(h.bundle));await screen.findByLabelText("文件备注");fireEvent.click(screen.getByRole("button",{name:"下载原文件"}));await waitFor(()=>expect(resolve).toBeDefined());rendered.rerender(view(h.bundle,b));await screen.findByLabelText("文件备注");await act(async()=>resolve(new Response("bytes",{headers:{etag:'"1"'}})));expect(create).not.toHaveBeenCalled();
});
it("clears the initialized local draft when content returns 401",async()=>{
 const h=setup();h.content(()=>json({code:"unauthorized",message:"expired",request_id:a},401));render(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"private draft"}});fireEvent.click(screen.getByRole("button",{name:"下载原文件"}));await screen.findByText("会话已失效，请重新登录。");expect(screen.queryByDisplayValue("private draft")).not.toBeInTheDocument();
});
it("retains a successful save when an older query resolves late",async()=>{
 const h=setup();render(view(h.bundle));fireEvent.change(await screen.findByLabelText("文件备注"),{target:{value:"saved"}});let resolve!:(r:Response)=>void;h.detail(()=>new Promise<Response>(r=>{resolve=r}));
 act(()=>{void h.bundle.queryClient.invalidateQueries({queryKey:queryKeys.files.detail(a)});});await waitFor(()=>expect(resolve).toBeDefined());fireEvent.click(screen.getByRole("button",{name:"保存备注和标签"}));await screen.findByText("保存成功。");await act(async()=>resolve(json(file())));expect(screen.getByLabelText("文件备注")).toHaveValue("saved");expect(h.bundle.queryClient.getQueryData<FileSource>(queryKeys.files.detail(a))?.version).toBe(2);
});
it("locks cursor buttons while the next file page is loading",async()=>{
 const h=setup();let resolve!:(r:Response)=>void;let calls=0;h.list(()=>++calls===1?json({items:[file()],next_cursor:"opaque-next"}):new Promise<Response>(r=>{resolve=r}));render(view(h.bundle));await waitFor(()=>expect(screen.getByRole("button",{name:"下一页文件"})).toBeEnabled());fireEvent.click(screen.getByRole("button",{name:"下一页文件"}));await waitFor(()=>expect(resolve).toBeDefined());expect(screen.getByRole("button",{name:"上一页文件"})).toBeDisabled();expect(screen.getByRole("button",{name:"下一页文件"})).toBeDisabled();await act(async()=>resolve(json({items:[file(b)],next_cursor:null})));await waitFor(()=>expect(screen.getByRole("button",{name:"上一页文件"})).toBeEnabled());
});
it("explicit relink stores target under its own identity and navigates to target",async()=>{
 const h=setup(),select=vi.fn();render(<UiProvider><QueryClientProvider client={h.bundle.queryClient}><FilesPage bundle={h.bundle} fileId={a} onSelectFile={select} /></QueryClientProvider></UiProvider>);await screen.findByLabelText("文件备注");fireEvent.change(screen.getByLabelText("重关联目标 UUID"),{target:{value:b}});fireEvent.click(screen.getByRole("button",{name:"读取重关联目标"}));fireEvent.click(await screen.findByRole("button",{name:"重关联到 beta.txt"}));fireEvent.click(await screen.findByRole("button",{name:"确认重关联"}));await waitFor(()=>expect(select).toHaveBeenCalledWith(b));expect(h.bundle.queryClient.getQueryData<FileSource>(queryKeys.files.detail(a))?.id).toBe(a);expect(h.bundle.queryClient.getQueryData<FileSource>(queryKeys.files.detail(b))?.id).toBe(b);
});

it("files deep link is authenticated and uses a strict return destination",async()=>{
 for(const value of ["/files",`/files/${a}`])expect(safeReturnTo(value)).toBe(value);
 for(const value of ["//evil/files","/files/invalid",`/files/${a}?next=//evil`,`/files/${a}/content`])expect(safeReturnTo(value)).toBe("/");
 const h=setup();const router=createAppRouter({history:createMemoryHistory({initialEntries:[`/files/${a}`]}),bundle:h.bundle});render(<UiProvider><App router={router} /></UiProvider>);await act(async()=>router.load());expect(await screen.findByLabelText("文件备注")).toHaveValue("original");
});
it("does not allow the same relink UUID with different casing",async()=>{
 const h=setup();const id="abcdefab-abcd-4abc-8abc-abcdefabcdef";h.rows[0]!.id=id;render(view(h.bundle,id));await screen.findByLabelText("文件备注");fireEvent.change(screen.getByLabelText("重关联目标 UUID"),{target:{value:id.toUpperCase()}});fireEvent.click(screen.getByRole("button",{name:"读取重关联目标"}));await screen.findByText("请输入不同来源的 UUID");expect(screen.queryByRole("button",{name:/重关联到/})).not.toBeInTheDocument();
});
it("accepts a pristine same-version projection when a tag was soft-deleted",async()=>{
 const h=setup();render(view(h.bundle));await screen.findByLabelText("文件备注");expect(screen.getByText(b,{exact:true})).toBeInTheDocument();act(()=>h.bundle.queryClient.setQueryData(queryKeys.files.detail(a),{...file(),tag_ids:[]}));await waitFor(()=>expect(screen.queryByText(b,{exact:true})).not.toBeInTheDocument());expect(screen.getByRole("button",{name:"保存备注和标签"})).toBeDisabled();
});
