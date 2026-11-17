import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createApiClient } from "@knoted/api-client";
import { describe, expect, it } from "vitest";
import { NoteAssociations } from "../../src/features/notes/links/NoteAssociations";
import { UiProvider } from "../../src/lib/ui-provider";
const noteId="00000000-0000-4000-8000-000000000001";
const tagId="00000000-0000-4000-8000-000000000002";
const targetId="00000000-0000-4000-8000-000000000003";
function json(value:unknown,status=200){return new Response(JSON.stringify(value),{status,headers:{"content-type":"application/json"}});}
function harness(fail=false,gate?:Promise<void>) {
 const requests:Request[]=[];let tags:unknown[]=[];let links:unknown[]=[];
 const api=createApiClient({baseUrl:"http://localhost/api/v1",fetch:async(input)=>{
 const request=input as Request;requests.push(request.clone());const path=new URL(request.url).pathname;
 if(request.method==="GET") {await gate;return json({items:path.endsWith("/links")?links:tags,next_cursor:null});}
 if(fail)return json({code:"dependency_unavailable",message:"服务暂不可用",request_id:noteId},503);
 if(path==="/api/v1/tags") return json({id:tagId,name:"研究",version:1,created_at:"2026-11-17T00:00:00Z",updated_at:"2026-11-17T00:00:00Z"},201);
 if(path.endsWith(`/tags/${tagId}`)) {tags=request.method==="PUT"?[{id:tagId,name:"研究",version:1}]:[];return new Response(null,{status:204});}
 if(path==="/api/v1/links") {const body=await request.json();const link={id:"link-1",...body,version:1,created_at:"2026-11-17T00:00:00Z"};links=[link];return json(link,201);}
 if(path.endsWith("/links/link-1")){links=[];return new Response(null,{status:204});}
 throw new Error(`${request.method} ${path}`);
 }});
 render(<UiProvider><QueryClientProvider client={new QueryClient({defaultOptions:{queries:{retry:false}}})}><NoteAssociations noteId={noteId} api={api}/></QueryClientProvider></UiProvider>);
 return requests;
}
describe("T32 note associations",()=>{
 it("creates and removes tags and stable identity links using the generated client",async()=>{
 const requests=harness();
 await waitFor(()=>expect(screen.getByRole("button",{name:"创建并添加标签"})).toBeEnabled());
 fireEvent.change(screen.getByLabelText("新标签名称"),{target:{value:"研究"}});
 fireEvent.click(screen.getByRole("button",{name:"创建并添加标签"}));
 expect(await screen.findByRole("button",{name:"移除标签 研究"})).toBeInTheDocument();
 fireEvent.click(screen.getByRole("button",{name:"移除标签 研究"}));
 await waitFor(()=>expect(screen.queryByRole("button",{name:"移除标签 研究"})).not.toBeInTheDocument());
 fireEvent.change(screen.getByLabelText("关联目标 ID"),{target:{value:targetId}});
 fireEvent.click(screen.getByRole("button",{name:"添加关联"}));
 expect(await screen.findByText(targetId)).toBeInTheDocument();
 fireEvent.click(screen.getByRole("button",{name:`移除关联 ${targetId}`}));
 await waitFor(()=>expect(screen.queryByText(targetId)).not.toBeInTheDocument());
 const created=requests.find(r=>r.method==="POST"&&new URL(r.url).pathname.endsWith("/links"));
 expect(await created?.json()).toEqual({source_type:"note",source_id:noteId,target_type:"note",target_id:targetId});
 expect(requests.some(r=>r.method==="DELETE"&&r.url.endsWith("/links/link-1?expected_version=1"))).toBe(true);
 });
 it("disables mutations while loading and preserves the input on server error",async()=>{
 let release!:()=>void;const gate=new Promise<void>(resolve=>{release=resolve});harness(true,gate);
 expect(screen.getByRole("button",{name:"添加关联"})).toBeDisabled();release();
 await waitFor(()=>expect(screen.getByRole("button",{name:"添加关联"})).toBeEnabled());
 fireEvent.change(screen.getByLabelText("关联目标 ID"),{target:{value:targetId}});
 fireEvent.click(screen.getByRole("button",{name:"添加关联"}));
 expect(await screen.findByRole("alert")).toHaveTextContent("服务暂不可用");
 expect(screen.getByLabelText("关联目标 ID")).toHaveValue(targetId);
 });
});
