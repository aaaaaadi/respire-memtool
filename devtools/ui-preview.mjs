import { createServer } from 'vite';
import { mkdir } from 'node:fs/promises';
await mkdir(new URL('../output/playwright/', import.meta.url), { recursive: true });
const categories=['家庭亲友','健康医疗','财务理财','饮食烹饪','住房家居','出行交通','购物消费','职业工作','任务计划','编程开发','学习成长','兴趣娱乐','电子游戏','运动健身','社交网络','旅行游历','宠物植物','情感心绪','习惯养成','灵感创意','技能库','经验库','踩坑录'];
const entries=[
{id:'root-dev',title:'编程开发',content:'开发事项',parent_id:null,importance:'important'},
{id:'topic',title:'工具专题',content:'日常工具维护',parent_id:'root-dev',importance:'important'},
{id:'leaf',title:'SSE 与代理兼容修复',content:'Windows 环境下的代理连接已兼容。',parent_id:'topic',importance:'important'},
{id:'root-family',title:'家庭亲友',content:'家庭事项',parent_id:null,importance:'important'},
{id:'family-leaf',title:'周末家人聚餐安排',content:'商量周末时间',parent_id:'root-family',importance:'trivial'},
...Array.from({length:473},(_,i)=>({id:'fixture-'+i,title:'示例记忆 '+(i+1),content:'仅用于界面验证的合成内容。',parent_id:'root-dev',importance:'important'}))
].map(entry=>({...entry,kind:'Context',updated_at:'2026-10-22T02:00:00Z'}));
const shim=`<script>window.__fixtureCalls=[];window.__TAURI__={core:{invoke:async(command,args)=>{window.__fixtureCalls.push({command,args});const entries=${JSON.stringify(entries)};switch(command){case 'get_app_config':return {update_check_on_start:false,reminder_enabled:false};case 'get_taxonomy':return ${JSON.stringify(categories.map((name,i)=>({id:name,name,domain:i<20?'human':'AI'})))};case 'list_recent':return {items:entries,scanned:478,truncated:false,category:null};case 'runtime_status':return {online:true,index_ready:true,pending:0,conflicts:0,message:'界面验证',index_hint:''};case 'list_reminders':return {items:[],overdue:0,today:0};case 'search_memory':return {mode:'recall',items:[{...entries[2],score:.92}],query:args.query};case 'extension_info':return {enabled:true,port:17650,token:'test-only-fixture'};default:throw new Error('界面验证不执行 '+command)}}},event:{listen:async()=>()=>{}},window:{getCurrentWindow:()=>({startDragging:async()=>{},hide:async()=>{}})}};</script>`;
const server=await createServer({server:{host:'127.0.0.1',port:1421,strictPort:true},plugins:[{name:'memtools-ui-fixture',transformIndexHtml(html){return html.replace('</head>',shim+'</head>')}}]});
await server.listen();console.log('UI fixture preview: http://127.0.0.1:1421');
