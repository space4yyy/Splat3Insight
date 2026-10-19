# Splat3 Insight

跨平台桌面应用：Splatoon 3 的 SplatNet 数据面板，附带透明的 OBS 数据卡片。

## 功能

- 数据面板：总览、对战、武器、场地、鲑鱼跑和对战记录，可按时间范围和模式筛选；对战记录里可以打开每一场的详情。
- OBS 数据卡片：自动、蛮颓比赛、X 比赛、鲑鱼跑四种，另有显示本机时间的时钟卡片，背景透明度可调。
- 添加并管理多个 Nintendo Account。
- 从 SplatNet 3 同步历史对战与打工记录，并在后台增量更新：OBS 打开时每 30 秒一次，其他时候每 5 分钟一次；窗口回到前台或点“立即同步”时马上同步。
- 战力变化只和同一条战力线比较：X 比赛按规则，蛮颓比赛（挑战）按武器，蛮颓比赛（开放）按模式，活动比赛按活动。开放和活动重新测定战力后，第一场从 +0 开始。
- 保留原始对战数据；SQLite 中的原始 JSON 经过 gzip 压缩并按记录去重。
- 登录凭据存储在操作系统凭据库中：macOS 钥匙串、Windows 凭据管理器。

## 下载与安装

在 [Releases](https://github.com/space4yyy/Splat3Insight/releases/latest) 下载最新版本：

- **macOS**：下载 `.dmg`，把 Splat3 Insight 拖进“应用程序”。应用没有经过 Apple 公证，第一次打开时如果提示无法验证开发者，到“系统设置 → 隐私与安全性”点“仍要打开”。
- **Windows**：下载 `-setup.exe` 运行。安装包没有代码签名，如果出现“Windows 已保护你的电脑”，点“更多信息 → 仍要运行”。

之后的新版本会在“设置 → 通用”里提示，点“更新到 x.y.z”即可下载安装并重启。应用不会自动安装更新，以免直播时中断 OBS 卡片。

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
make test       # 运行 Rust 与前端测试
make fmt-check  # 检查 Rust 格式
make check      # 检查 Rust crates
make build      # 构建桌面安装包
```

每次推送后，GitHub Actions 会构建 Windows 安装包（`.github/workflows/build-windows.yml`）。

## 发布

1. 第一次发布前运行 `make release-signing`，生成自动更新签名密钥和 macOS 发布证书，按提示把内容填进仓库的 Actions secrets，并另外备份生成的目录。丢了更新私钥，已安装的版本就收不到后续更新。
2. 在 main 上运行 `make release VERSION=0.3.0`：修改版本号、提交并打上 `v0.3.0` 标签（不推送）。
3. 推送 main 和标签：`git push origin main v0.3.0`。GitHub Actions（`.github/workflows/release.yml`）会打包 macOS 和 Windows 版，上传到同名的 Release 草稿。
4. 下载草稿里的安装包检查无误后，在网页上发布草稿。发布后，已安装的应用会按这个 Release 里的 `latest.json` 收到更新。

macOS 发布流程会把现有 Secrets 中的证书导入构建机的临时钥匙串，验证签名后再打包，并在结束时清理。项目生成的自签名证书可直接使用，无需更换证书或更新密钥。修复发布流程后，需要用包含修复的新版本标签触发构建；重新运行旧标签的任务仍会使用旧代码。

## OBS 设置

1. 应用启动后会自动运行 OBS 服务，可以在“其他工具”页停止或重新启动。
2. 在“其他工具”页选择卡片类型，复制 OBS 地址，例如自动卡片是 `http://127.0.0.1:17353/overlay`。
3. 在 OBS 添加“浏览器”来源，填入这个地址，宽高设为 **1280 × 320**（或其他 4:1 尺寸）；时钟卡片的地址是 `http://127.0.0.1:17353/clock`，宽高设为 **576 × 192**。
4. 需要背景时，调好透明度，把页面上的 CSS 粘贴到浏览器来源属性的“自定义 CSS”。

卡片按来源的宽高等比缩放，按来源的分辨率绘制：在画面里缩小保持清晰，放大超过来源尺寸会略糊。

若端口 17353 已被占用，应用会显示服务错误状态；关闭冲突程序后重新启动服务。

## 数据与隐私

登录 SplatNet 3 需要第三方服务 [nxapi](https://github.com/samuelthomas2774/nxapi-znca-api) 生成登录参数：登录和刷新令牌时，Nintendo 账号的 id_token、Nintendo Switch Online 应用接口（Coral）的令牌，以及和 Coral 收发的数据会发送给 nxapi。每次添加账号前，应用都会先说明这一点，确认后才开始登录。

账号元数据和压缩后的对战/打工历史存放在操作系统为应用分配的数据目录中。Nintendo 与 SplatNet 凭据放在系统凭据库，不保存在仓库。应用日志避免写入密码、登录链接和令牌。

每个账号的历史数据可以在“设置”的账号列表里导出，导出文件也可以再导入（例如换电脑后），本地已有的记录不会重复写入。删除账号时默认只移除登录信息，本地记录保留；勾选“同时删除本地数据”才会一起删除。导出的文件不会被删除。
