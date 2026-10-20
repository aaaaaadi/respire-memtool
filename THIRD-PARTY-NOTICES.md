# 第三方组件声明

memTools 以 MIT 许可证发布。应用在构建和运行时会使用下列第三方开源组件，这些组件分别遵循其自身的许可证。各组件的完整许可证文本随上游分发包提供，也可在上游仓库中查阅。

## Rust 依赖

| 组件 | 许可证 | 上游 |
| --- | --- | --- |
| tauri | MIT 或 Apache-2.0 | https://github.com/tauri-apps/tauri |
| tauri-build（构建时） | MIT 或 Apache-2.0 | https://github.com/tauri-apps/tauri |
| tauri-plugin-global-shortcut | MIT 或 Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| tauri-plugin-single-instance | MIT 或 Apache-2.0 | https://github.com/tauri-apps/plugins-workspace |
| serde | MIT 或 Apache-2.0 | https://github.com/serde-rs/serde |
| serde_json | MIT 或 Apache-2.0 | https://github.com/serde-rs/json |
| ureq | MIT 或 Apache-2.0 | https://github.com/algesten/ureq |
| toml | MIT 或 Apache-2.0 | https://github.com/toml-rs/toml |
| tiny_http | MIT 或 Apache-2.0 | https://github.com/tiny-http/tiny-http |
| rusqlite | MIT | https://github.com/rusqlite/rusqlite |
| flate2 | MIT 或 Apache-2.0 | https://github.com/rust-lang/flate2-rs |
| sha2 | MIT 或 Apache-2.0 | https://github.com/RustCrypto/hashes |
| chrono | MIT 或 Apache-2.0 | https://github.com/chronotope/chrono |
| arboard | MIT 或 Apache-2.0 | https://github.com/1Password/arboard |
| windows-sys（仅 Windows） | MIT 或 Apache-2.0 | https://github.com/microsoft/windows-rs |

`rusqlite` 默认启用 bundled SQLite，其内置的 SQLite 源码属于公有领域（public domain）。

## 前端依赖

| 组件 | 许可证 | 上游 |
| --- | --- | --- |
| three | MIT | https://github.com/mrdoob/three.js |
| 3d-force-graph | MIT | https://github.com/vasturiano/3d-force-graph |
| three-spritetext | MIT | https://github.com/vasturiano/three-spritetext |
| vite（构建时） | MIT | https://github.com/vitejs/vite |
| @tauri-apps/cli（构建时） | MIT 或 Apache-2.0 | https://github.com/tauri-apps/tauri |

## 可视化资源

`src/graph/assets/brain2.container.ply` 是本仓库随附的 3D 可视化资源，按本项目的 MIT 许可证分发。

## 说明

- 表中“MIT 或 Apache-2.0”表示上游同时提供两种许可证，使用者可任选其一。
- 本文件不替代上游许可证文本；分发或再发布时请保留上游包内的版权与许可证声明。
- 若使用的版本与上表不同，请以实际依赖版本所附带的许可证为准。
