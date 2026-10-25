import { createServer } from 'vite';
const nodes = [
 {id:'root',name:'编程开发',x:0,y:100,z:0,parent_id:null},
 {id:'xml-1',name:'XML 注释配色',x:-190,y:0,z:100,parent_id:'root',tags:['IDEA']},
 {id:'ts-2',name:'TypeScript 前端',x:190,y:0,z:100,parent_id:'root',tags:['前端']},
 {id:'java-3',name:'Java 服务接口',x:0,y:-160,z:0,parent_id:'root',tags:['后端']},
 {id:'safe',name:'<img src=x onerror=alert(1)>',x:0,y:0,z:-200,parent_id:'root'}
].map(node=>({...node,category:'编程开发',kind:'knowledge',importance:'important',content_head:'仅用于本地交互检查的合成记忆',recall_count:0}));
const snapshot={nodes,links:nodes.filter(n=>n.parent_id).map(n=>({id:'p_'+n.id,source:n.parent_id,target:n.id,type:'parent',directed:true})),cursor:{seq:0,qid:0},stats:{nodes:nodes.length,links:nodes.length-1}};
const shim='<script>window.__fixtureCalls=[];window.__TAURI__={core:{invoke:async(command,args)=>{window.__fixtureCalls.push({command,args});switch(command){case "get_app_config":return {update_check_on_start:false,reminder_enabled:false};case "get_taxonomy":return [];case "runtime_status":return {online:true,index_ready:true};case "graph_snapshot":return '+JSON.stringify(snapshot)+';case "graph_events_since":return {ops:[],cursor:{seq:0,qid:0}};case "set_graph_window":return null;case "get_targets":return {items:[]};case "get_processor_sources":return {options:[]};default:return null;}}},event:{listen:async()=>()=>{}},window:{getCurrentWindow:()=>({startDragging:async()=>{},hide:async()=>{window.__fixtureHidden=true}})}};</script><script type="module">import {GraphView} from "/src/graph/graphView.js";const emit=GraphView.prototype._emitStatus;GraphView.prototype._emitStatus=function(...args){window.__graphView=this;return emit.apply(this,args)};</script>';
const server=await createServer({server:{host:'127.0.0.1',port:1423,strictPort:true},plugins:[{name:'graph-fixture',transformIndexHtml(html){return html.replace('</head>',shim+'</head>')}}]});
await server.listen();console.log('Graph fixture: http://127.0.0.1:1423');
