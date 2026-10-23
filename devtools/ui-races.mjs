async (page) => {
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  await page.goto('http://127.0.0.1:1421/');
  await page.getByRole('button',{name:'记录',exact:true}).waitFor();
  await page.evaluate(() => {
    const original=window.__TAURI__.core.invoke;
    window.__snapshotReads=0;
    window.__TAURI__.core.invoke=async(command,args)=>{
      if(command==='submit_memory') return {ok:true,action:'stored',id:'new-fixture',title:'并发新增合成记忆'};
      if(command!=='list_recent') return original(command,args);
      const response=await original(command,args);
      window.__snapshotReads++;
      if(window.__snapshotReads===1) await new Promise(resolve=>window.__releaseSnapshot=resolve);
      else response.items.push({id:'new-fixture',title:'并发新增合成记忆',content:'只在浏览器中模拟'});
      return response;
    };
  });
  await page.getByRole('button',{name:'最近记忆',exact:true}).click();
  await page.waitForFunction(()=>Boolean(window.__releaseSnapshot));
  await page.getByRole('button',{name:'记录',exact:true}).click();
  await page.getByRole('button',{name:'日记 trivial',exact:true}).click();
  await page.locator('#memory-input').fill('仅用于浏览器验证的合成新增记忆');
  await page.evaluate(()=>window.__memtools.submit());
  await page.evaluate(()=>window.__releaseSnapshot());
  await page.getByRole('button',{name:'最近记忆',exact:true}).click();
  await page.waitForFunction(()=>document.getElementById('browse-status').textContent.includes('全库 479 条'));
  check(await page.evaluate(()=>window.__snapshotReads)===2,'读取中写入成功后未重新读取快照');
  await page.goto('http://127.0.0.1:1421/');
  await page.getByRole('button',{name:'记录',exact:true}).waitFor();
  await page.evaluate(()=>{
    const original=window.__TAURI__.core.invoke;
    window.__TAURI__.core.invoke=(command,args)=>command==='list_recent'?Promise.reject(new Error('模拟全量列表不可用')):original(command,args);
  });
  await page.getByRole('navigation').getByRole('button',{name:'搜索',exact:true}).click();
  await page.getByRole('button',{name:'语义',exact:true}).click();
  await page.getByRole('searchbox',{name:'搜索全部记忆'}).fill('代理');
  await page.locator('#search-btn').click();
  await page.waitForFunction(()=>document.getElementById('search-status').textContent.includes('语义匹配 1 条'));
  check(await page.locator('#search-list .mem-title').last().textContent()==='SSE 与代理兼容修复','全量列表不可用导致语义结果丢失');
  return {checks:['snapshot-invalidated-during-write','semantic-survives-list-failure']};
}
