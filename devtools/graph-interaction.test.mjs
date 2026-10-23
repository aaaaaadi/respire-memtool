import test from 'node:test';
import assert from 'node:assert/strict';
import { pickProjectedNode, isGraphClick, searchGraphNodes } from '../src/graph/interaction.js';
const node=(id,x,y,z=10,radius=2)=>({node:{id},x,y,z,radius});
test('small nodes accept nearby clicks but not distant background',()=>{
 const points=[node('a',50,50)];
 assert.equal(pickProjectedNode(points,{x:60,y:50})?.id,'a');
 assert.equal(pickProjectedNode(points,{x:64,y:50}),null);
});
test('nearby halos choose the closest node and visible overlap chooses the front',()=>{
 assert.equal(pickProjectedNode([node('farther',45,50),node('nearest',53,50)],{x:55,y:50})?.id,'nearest');
 assert.equal(pickProjectedNode([node('back',50,50,100,8),node('front',50,50,10,8)],{x:50,y:50})?.id,'front');
 assert.equal(pickProjectedNode([node('hidden',50,50,-10)],{x:50,y:50}),null);
});
test('minor pointer jitter remains a click while real drags and other buttons do not',()=>{
 const start={x:20,y:20,pointerId:1,button:0,maxDistance:0};
 assert.equal(isGraphClick(start,{x:23,y:22,pointerId:1,button:0}),true);
 assert.equal(isGraphClick({...start,maxDistance:20},{x:20,y:20,pointerId:1,button:0}),false);
 assert.equal(isGraphClick(start,{x:33,y:20,pointerId:1,button:0}),false);
 assert.equal(isGraphClick(start,{x:20,y:20,pointerId:2,button:0}),false);
 assert.equal(isGraphClick({...start,button:2},{x:20,y:20,pointerId:1,button:2}),false);
});
test('node search supports title category tag and id without HTML interpretation',()=>{
 const nodes=[{id:'xml-1',name:'XML 注释',category:'编程开发',tags:['IDEA'],content_head:'配色问题'},
 {id:'js-2',name:'JavaScript',category:'编程开发',tags:['前端']},{id:'safe',name:'<img onerror=alert(1)>'}];
 assert.deepEqual(searchGraphNodes(nodes,'idea XML').map(n=>n.id),['xml-1']);
 assert.deepEqual(searchGraphNodes(nodes,'JS-2').map(n=>n.id),['js-2']);
 assert.deepEqual(searchGraphNodes(nodes,'前端').map(n=>n.id),['js-2']);
 assert.deepEqual(searchGraphNodes(nodes,'不存在'),[]);
 assert.deepEqual(searchGraphNodes(nodes,'   '),[]);
 assert.equal(searchGraphNodes(nodes,'<img')[0].name,'<img onerror=alert(1)>');
});
