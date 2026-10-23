# 图谱节点交互与验证

小节点在缩放后也保留至少 12 CSS 像素的点击半径；轻微鼠标抖动仍可选中，实际拖动保持旋转。重叠模型优先前景，扩展点击区优先最近节点中心。

鼠标悬停提示完整标题、分类和标签。点击锁定高亮与详情并暂停自转，经过其他节点不改变锁定。取消选择、关闭详情、删除节点或重载快照会清除相应状态。

顶部搜索匹配当前已加载图谱节点的标题、分类、标签、ID 与正文预览，支持多个关键词。最多先展示 30 个候选并提示总数；点击候选居中放大。它不代替全库记忆检索。输入框 Enter 选择首项、下键进入候选，候选上下键移动；Escape 关闭候选，不触发全局隐藏快捷键。

## 自动回归

安装项目依赖后：

~~~powershell
node --test devtools/extension-settings.test.mjs devtools/memory-browser.test.mjs devtools/graph-interaction.test.mjs
npm run build
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib
~~~

## 浏览器交互检查

启动只使用合成节点和 Tauri 接口替身的页面：

~~~powershell
node devtools/graph-preview.mjs
~~~

另一个终端使用 Playwright CLI（需已配置可用浏览器）：

~~~powershell
npx --package @playwright/cli playwright-cli -s=graph-check open http://127.0.0.1:1423/
npx --package @playwright/cli playwright-cli -s=graph-check run-code --filename devtools/graph-ui-check.mjs
npx --package @playwright/cli playwright-cli -s=graph-check close
~~~

覆盖扩大点击、悬停标题、锁定高亮、搜索定位、自转暂停、取消/关闭、标题按安全文本展示、空结果、轻微抖动、拖动、键盘、搜索 Escape、删除节点与窄窗口。截图写入 output/playwright/。验证不连接真实 Runtime，不读取或写入实际记忆；结束后关闭预览进程。
