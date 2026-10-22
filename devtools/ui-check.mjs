async (page) => {
  await page.goto('http://127.0.0.1:1421/');
  const settle = () => page.evaluate(async () => {
    await new Promise(requestAnimationFrame);
    await Promise.all(document.getAnimations().filter(a => a.effect.getComputedTiming().iterations !== Infinity).map(a => a.finished.catch(() => {})));
  });
  const out = 'output/playwright';
  const check = (ok, message) => { if (!ok) throw new Error(message); };
  const waitText = (id, text) => page.waitForFunction(({id,text}) => document.getElementById(id)?.textContent.includes(text), {id,text});
  await page.setViewportSize({width:480,height:560});
  await page.emulateMedia({colorScheme:'light'});
  await page.getByRole('button',{name:'最近记忆',exact:true}).click();
  await waitText('browse-status','全库 478 条');
  const positions = () => page.locator('#browse-cats .cat-chip').evaluateAll(nodes => nodes.map(node=>({x:node.getBoundingClientRect().x,y:node.getBoundingClientRect().y})));
  const before=await positions();
  const columns=before.filter(p=>p.y===before[0].y).length;
  check(columns===5,'480px 分类不是固定五列');
  await page.getByRole('button',{name:'编程开发 476',exact:true}).click();
  await waitText('browse-status','476 条');
  const after=await positions();
  check(before.every((p,i)=>Math.abs(p.x-after[i].x)<.5&&Math.abs(p.y-after[i].y)<.5),'分类选中后发生排列跳动');
  await page.getByRole('button',{name:'任务计划 0',exact:true}).click();
  await waitText('browse-status','当前范围 0 条');
  check(await page.getByText('当前分类暂无匹配记忆。',{exact:true}).isVisible(),'空分类没有友好空状态');
  await page.getByRole('button',{name:'全部 478',exact:true}).click();
  await page.getByRole('combobox',{name:'浏览范围'}).selectOption('20');
  await waitText('browse-status','显示 20 条');
  await page.getByRole('combobox',{name:'浏览范围'}).selectOption('0');
  await waitText('browse-status','显示 478 条');
  await page.screenshot({path:out+'/browse-light.png'});
  await page.getByRole('navigation').getByRole('button',{name:'搜索',exact:true}).click();
  await page.getByRole('searchbox',{name:'搜索全部记忆'}).fill('代理');
  await waitText('search-status','匹配 1 条');
  const titles=await page.locator('#search-list .mem-title').allTextContents();
  check(JSON.stringify(titles)===JSON.stringify(['编程开发','工具专题','SSE 与代理兼容修复']),'搜索树祖先路径或命中集合不正确：'+JSON.stringify(titles));
  await page.screenshot({path:out+'/search-tree-light.png'});
  await page.getByRole('group',{name:'搜索结果展示方式'}).getByRole('button',{name:'列表',exact:true}).click();
  check(await page.locator('#search-list .mem-item').count()===1,'列表模式包含了额外路径节点');
  await page.getByRole('group',{name:'搜索结果展示方式'}).getByRole('button',{name:'树形',exact:true}).click();
  await page.getByRole('button',{name:'语义',exact:true}).click();
  await waitText('search-status','语义匹配 1 条');
  await page.getByRole('button',{name:'关键词',exact:true}).click();
  await waitText('search-status','全库关键词匹配 1 条');
  const reads=await page.evaluate(()=>window.__fixtureCalls.filter(call=>call.command==='list_recent').length);
  check(reads===1,'即时筛选重复读取 Runtime 快照：'+reads);
  const contrasts=[];
  for(const scheme of ['light','dark']){
    await page.emulateMedia({colorScheme:scheme});
    await settle();
    const results=await page.locator('#search-btn, .view-tab.active, #search-kind .mini-btn.active, #search-view-mode .mini-btn.active').evaluateAll(nodes=>nodes.map(node=>{const s=getComputedStyle(node);return {label:node.textContent.trim(),foreground:s.color,background:s.backgroundColor};}));
    const channel=s=>s.match(/[\d.]+/g).slice(0,3).map(Number).map(v=>{v/=255;return v<=.04045?v/12.92:Math.pow((v+.055)/1.055,2.4)});
    const light=s=>{const c=channel(s);return c[0]*.2126+c[1]*.7152+c[2]*.0722};
    for(const result of results){
      const a=light(result.foreground),b=light(result.background);const ratio=(Math.max(a,b)+.05)/(Math.min(a,b)+.05);
      check(ratio>=4.5,scheme+' '+result.label+' 对比度不足：'+ratio);contrasts.push({scheme,label:result.label,ratio});
    }
    await page.screenshot({path:out+'/search-tree-'+scheme+'.png'});
  }
  await page.emulateMedia({colorScheme:'light'});
  await page.getByRole('button',{name:'扩展',exact:true}).click();
  await page.getByRole('button',{name:'复制令牌',exact:true}).waitFor({state:'visible'});
  await settle();
  await page.screenshot({path:out+'/extension-light.png'});
  const frame=await page.locator('.shell').evaluate(node=>{const s=getComputedStyle(node);return {radius:s.borderRadius,border:s.borderTopWidth,width:document.documentElement.scrollWidth};});
  check(frame.radius==='0px'&&frame.border==='0px','CSS 重复外壳圆角仍然存在');
  check(frame.width<=480,'480px 窗口横向溢出');
  await page.getByRole('button',{name:'关闭',exact:true}).click();
  await page.setViewportSize({width:400,height:560});
  await page.getByRole('button',{name:'最近记忆',exact:true}).click();
  const narrow=await positions();check(narrow.filter(p=>p.y===narrow[0].y).length===4,'窄窗口未保持固定四列');
  await page.setViewportSize({width:480,height:560});
  return ({checks:['all-478','fixed-columns','empty-category','scope-20-all','tree-ancestor-path','list-tree-toggle','semantic-preserved','one-snapshot-only','single-css-frame','narrow-columns'],contrasts,frame});
}
