# Splat3 Insight

跨平台桌面应用，为 OBS 提供透明的 Splatoon 3 对战信息卡片。

## 功能

- 添加并管理多个 Nintendo Account。
- 从 SplatNet 3 同步历史对战与打工记录，并在后台增量更新：OBS 正在显示卡片时每 30 秒一次，其他时候每 5 分钟一次；窗口回到前台或点“立即同步”时马上同步。
- 在 OBS Browser Source 中显示最近一场符合条件的对战卡片。
- 按 X 赛规则、蛮颓挑战武器或蛮颓开放队伍分别计算战力变化。
- 保留原始对战数据；SQLite 中的原始 JSON 经过 gzip 压缩并按记录去重。
- 登录凭据存储在操作系统凭据库中：macOS Keychain、Windows Credential Manager。

## 开发环境

- Rust stable 与 Cargo
- Node.js 和 npm
- Tauri 2 所需的操作系统开发组件

安装桌面端依赖：

```sh
npm --prefix desktop install
```

启动桌面开发版：

```sh
make dev
```

也可以直接运行：

```sh
npm --prefix desktop run tauri -- dev
```

macOS 上可以先运行一次 `make dev-signing`：开发版会用本机固定的证书签名，钥匙串的“始终允许”在重新编译后依然有效，不用每次输入密码。

## 构建与检查

```sh
make test       # 运行 core 与桌面端 Rust 测试
make fmt-check  # 检查 Rust 格式
make check      # 检查 Rust crates
make build      # 构建桌面安装包
```

## OBS 设置

1. 启动应用并在卡片预览区域启动 OBS 服务。
2. 在 OBS 添加“浏览器”来源，地址设为 `http://127.0.0.1:17353/overlay`。
3. 推荐尺寸为 **560 × 100**。尺寸较小时卡片会按比例缩小；不会拉伸。

服务由桌面应用管理。若端口 17353 已被占用，应用会显示服务错误状态；关闭冲突程序后重新启动服务。

## 数据与隐私

账号元数据和压缩后的对战/打工历史存放在操作系统为应用分配的数据目录中。Nintendo 与 SplatNet 凭据放在系统凭据库，不保存在仓库。应用日志避免写入密码、登录链接和令牌。

每个账号的历史数据可通过应用中的“导出”功能导出。删除账号会同时删除该账号在应用本地保存的历史记录和导出文件。
