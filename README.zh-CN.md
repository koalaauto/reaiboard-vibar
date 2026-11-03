# Vibar

**ReAI-Vibe-Board 键盘的非官方 macOS 菜单栏客户端。**
蓝牙直连键盘,重新映射每一个旋钮、按键和模式拨杆,不需要安装官方 App。

[English](README.md) | 简体中文

<p align="center">
  <img src="https://ai-board.reai.com/img/aiboard/hardware-unibody.webp" alt="ReAI-Vibe-Board 键盘" width="560">
  <br>
  <sub>产品图 © ReAI,链接自<a href="https://ai-board.reai.com/en">官网</a>。</sub>
</p>

> **Vibar 是社区项目,与 ReAI 公司没有隶属关系。**
> "ReAI"、"ReAI-Vibe-Board" 等名称归其各自所有者,这里只用来说明 Vibar 适配的硬件。

## 这块键盘

[ReAI-Vibe-Board](https://ai-board.reai.com/en)(也写作 ReAI Vibe Board、REAI Vibe Board、AI-Board)是一块围绕 AI 工作流设计的语音优先机械键盘。按厂商的介绍:

- CNC 铝合金一体机身,可旋转也可按下的金属旋钮,6 个按键
- 一个实体**三段拨杆 CHAT、PLAN、YOLO**,定位是 AI 权限开关,先问再做、只提方案、全自动
- 双麦克风阵列,USB-C 与蓝牙自动切换
- 开放 SDK:[`ReAI-com/reai-board-sdk`](https://github.com/ReAI-com/reai-board-sdk)

| | |
|---|---|
| USB | vendor id `0x363C`,product id `0xED20`,产品名 `REAI Vibe Board` |
| 蓝牙 | 广播名 `REAI_VB_xxxxxxxx`,厂商 GATT 服务 `FE60` 到 `FE63` |

## 为什么需要 Vibar

走蓝牙时这块键盘**不是**普通的蓝牙键盘,它通过厂商自定义的 GATT 服务通信,在「系统设置 → 蓝牙」里配对没有用。必须有一个程序订阅它的通知,再把旋钮、按键、拨杆变成真正的操作。厂商提供了桌面 App,Vibar 是基于厂商 MIT 许可 SDK 的另一个选择,小、开源、好改。

<p align="center">
  <img src="docs/images/settings.png" alt="Vibar 设置窗口,键位页" width="640">
</p>

## 功能

- **蓝牙直连**,键盘休眠唤醒后自动重连
- **全部可重新映射**:旋钮三个输入(逆时针、顺时针、按下)和 6 个按键,可设为快捷键、按住说话、媒体键、命令或无操作
- **三层绑定,对应拨杆三档。** 先设一套默认,再只覆盖 CHAT、YOLO、PLAN 里不同的键,还能在拨杆拨入或拨出时触发动作
- **设置窗口带实时显示**:按下真实的键,窗口里对应位置会亮起来
- **键盘休眠时间**可在应用里调整,出厂的 10 分钟偏短
- **把键盘用作系统麦克风**,通过虚拟音频设备实现,只在有 App 录音时才打开
- **一键把键盘让给另一台电脑**:先断开,再在另一台上连接
- 只有菜单栏,支持深色模式,可开机自启

界面目前是中文。欢迎贡献翻译,见 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 状态

版本 **0.1.0**,仅 macOS。厂商提供 Windows 版,Vibar 暂不支持。

| 真机验证过 | |
|---|---|
| 环境 | macOS 26.6,Apple Silicon,键盘固件 1.75,蓝牙 |
| 已确认可用 | 扫描和连接、完整事件流(旋钮、按键、拨杆)、休眠后重连、旋钮控制系统音量、读写休眠时间、虚拟麦克风驱动安装 |
| **尚未验证** | 拨杆分层在真机上的效果、旋钮方向校准、麦克风声音进入录音 App、让给第二台电脑、开机自启 |

Intel Mac 和其他固件版本没有测过,欢迎反馈。

## 安装

暂时没有预编译版本,请从源码构建:

```bash
git clone https://github.com/koalaauto/reaiboard-vibar.git
cd reaiboard-vibar
scripts/bundle.sh --run      # 构建 build/Vibar.app 并启动
```

要求:macOS 12 以上、Rust 1.87 以上、`cmake`(编译虚拟麦克风驱动,`brew install cmake`)。

### 权限与签名

Vibar 需要两项权限:

- **蓝牙**,用来和键盘通信
- **辅助功能**,用来替你按键。没授权时菜单里会出现「需要辅助功能权限…」

macOS 靠 **bundle id 加代码签名**记住授权。`scripts/bundle.sh` 会用钥匙串里第一个代码签名证书签名;一个都没有时退回 ad-hoc 签名,而 **ad-hoc 签名每次构建都不同,每次重新构建都要重新授权**。详情和避免办法见 [docs/signing-and-permissions.md](docs/signing-and-permissions.md)。

### 第一次使用

1. 拔掉 USB。键盘插着 USB 时不会广播蓝牙。
2. **不要**在「系统设置 → 蓝牙」里配对。配过的话先在那里移除,否则系统会一直占着连接,Vibar 找不到它。
3. 按任意键唤醒键盘,连上后菜单栏图标变成实心。
4. 不要同时运行官方 App,同一时间只有一个程序能占用键盘。

## 使用

菜单栏图标 → **设置…**。开箱默认键位:

| 输入 | 默认 |
|---|---|
| 旋钮 逆时针 / 顺时针 / 按下 | `↑` / `↓` / `Return` |
| Tab、New、Esc、Action、Enter | `Tab`、`⌘N`、`Esc`、`⌘Return`、`Return` |
| 语音键 | 按住 `F18` |

把你的语音输入工具的按住说话快捷键设成 **按住 F18**,语音键就成了说话键。所有设置也保存在一个 TOML 文件里,见 [docs/configuration.md](docs/configuration.md)。

## 文档

| | |
|---|---|
| [docs/architecture.md](docs/architecture.md) | 模块、线程、数据流,改代码从这里开始 |
| [docs/configuration.md](docs/configuration.md) | 配置文件参考 |
| [docs/hardware.md](docs/hardware.md) | 我们对键盘的了解:输入、蓝牙行为、休眠、麦克风 |
| [docs/design-principles.md](docs/design-principles.md) | 界面文字和图标规范 |
| [docs/signing-and-permissions.md](docs/signing-and-permissions.md) | 签名、权限、发布 |
| [CONTRIBUTING.md](CONTRIBUTING.md) | 环境、本地 CI、约定 |
| [AGENTS.md](AGENTS.md) | 给 AI 编程助手的说明 |

## 安全说明

除了你自己修改的**休眠时间**,Vibar 对键盘只读,不会碰按键配置、固件升级和工厂命令。安装虚拟麦克风需要管理员密码,并会重启 `coreaudiod`,系统声音会中断一两秒。安装前会先征得你同意,也可以在设置窗口里卸载驱动。

## 致谢与许可

- [`reai-board-sdk`](https://github.com/ReAI-com/reai-board-sdk)(ReAI,MIT)承担蓝牙和协议部分。其中的 `msbc-decoder` 是 **LGPL-2.1-or-later**,见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。
- 使用了 [tao](https://github.com/tauri-apps/tao)、[wry](https://github.com/tauri-apps/wry)、[tray-icon](https://github.com/tauri-apps/tray-icon)、[enigo](https://github.com/enigo-rs/enigo)。

Vibar 本身以 [MIT 许可证](LICENSE) 发布。
