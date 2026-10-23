import test from "node:test";
import assert from "node:assert/strict";
import { selectMemories, memoryForest } from "../src/memory-browser.js";
const entries = [
 {id:"leaf", title:"代理修复", content:"SSE 已兼容 Windows", parent_id:"branch"},
 {id:"other", title:"另一事项", content:"", parent_id:null},
 {id:"branch", title:"工具专题", content:"", parent_id:"root"},
 {id:"root", title:"编程开发", content:"", parent_id:null},
];
test("empty taxonomy category yields an empty view rather than a tree lookup error", () => {
 assert.deepEqual(selectMemories(entries, {category:"任务计划"}), []);
});
test("category selection follows real ancestors and includes the category node", () => {
 assert.deepEqual(selectMemories(entries, {category:"编程开发"}).map(x=>x.id), ["leaf","branch","root"]);
});
test("quick search matches title content or id with case insensitive terms", () => {
 assert.deepEqual(selectMemories(entries,{query:"sse windows"}).map(x=>x.id), ["leaf"]);
 assert.deepEqual(selectMemories(entries,{query:"BRANCH"}).map(x=>x.id), ["branch"]);
});
test("all scope does not cap results at 200", () => {
 const many=Array.from({length:478},(_,i)=>({id:String(i),title:String(i)}));
 assert.equal(selectMemories(many,{limit:0}).length,478);
 assert.equal(selectMemories(many,{limit:20}).length,20);
});
test("tree search keeps ancestors but excludes unrelated siblings", () => {
 const forest=memoryForest(entries,[entries[0]]);
 assert.equal(forest.length,1);
 assert.equal(forest[0].entry.id,"root");
 assert.equal(forest[0].matched,false);
 assert.equal(forest[0].children[0].entry.id,"branch");
 assert.equal(forest[0].children[0].children[0].entry.id,"leaf");
 assert.equal(forest[0].children[0].children[0].matched,true);
});
test("orphaned and cyclic parents cannot hide results or recurse forever", () => {
 const rows=[{id:"a",parent_id:"b"},{id:"b",parent_id:"a"},{id:"c",parent_id:"missing"}];
 const forest=memoryForest(rows,rows);
 const found=[];const walk=nodes=>{for(const n of nodes){found.push(n.entry.id);walk(n.children)}};walk(forest);
 assert.deepEqual(found.sort(),["a","b","c"]);
});
