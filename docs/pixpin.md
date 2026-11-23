# PixPin 3.6.6.0 功能清单与 Rshot 实现边界

核对基准是官方 3.6.6.0 正式版日志、用户手册与本机 macOS 3.6.6.0 构建 9090。手册有持续更新，下文对明确标注测试版的行为保留其版本标签。Windows 行为根据官方文档记录。二进制检查确认应用身份和框架，以下交互以文档为依据，实机行为仍需按条验证。[正式版日志](https://pixpin.cn/changelog/3.6.6.0/) · [用户手册](https://pixpin.cn/docs)

分析本机分发物中各模块的调用路径，见 [PixPin 实现原理](pixpin-implementation.md)。

## 入口与截图

| 功能 | PixPin 行为 |
| --- | --- |
| 入口 | 后台常驻，Windows 托盘、macOS 菜单栏、可停靠悬浮球、全局键盘与鼠标动作。悬浮球支持拖放贴图、按钮定制、紧凑与展开、自动隐藏。 |
| 区域模式 | 矩形、自由手绘、折线多边形、多窗口合成。支持在已有选区上增加或扣除区域，多窗口合成可移除窗口间背景。 |
| 自动定位 | 鼠标下的窗口或 UI 元素可直接选中，支持仅窗口、关闭检测和边距。手动框选后可移动、调整边界、用键盘微调，并跨显示器截图。 |
| 定位辅助 | 像素放大镜、十字线、选区中心线、尺寸坐标、三分网格。可刷新底图、切换鼠标指针输出、设置圆角、阴影、边框。 |
| 取色 | 在放大镜或标注工具中取色，可复制坐标或 RGB、HEX 等颜色表示，并配置自定义格式。 |
| 精确复用 | 固定尺寸、位置、自定义延时、复用上次选区，以及可管理的尺寸和位置预设。图像历史与选区历史分别保存，可从历史恢复。 |
| 输出与联动 | 复制、贴图、手动保存、快速保存、打印、OCR、扫码、编辑、长截图、录屏。保存动作可按规则连带复制文件或路径，复制、贴图、识别也可分别触发自动保存。 |

[基础截图](https://pixpin.cn/docs/screenshot/basics) · [截图模式](https://pixpin.cn/docs/screenshot/modes) · [自定义截图](https://pixpin.cn/docs/screenshot/custom) · [取色](https://pixpin.cn/docs/screenshot/color-picker) · [悬浮球](https://pixpin.cn/docs/getting-started/floating-ball)

## 标注与文件输出

- 标注可用于截图、图片贴图和独立图片编辑。工具包括矩形、椭圆等形状，直线及折线、波浪线、箭头、自由画笔、荧光笔、文本、水印、序号、聚光灯、放大镜、马赛克、自动马赛克及橡皮擦。智能擦除属于会员功能。
- 标注对象可以重新选中后修改、拖动复制、删除、撤销和重做。可管理工具栏层级及顺序，并记住组内上次使用的工具。
- 图像输出包括 PNG、TIFF、BMP、JPEG、WebP、AVIF，以及会员 PDF。可使用尺寸与纵横比预设、压缩质量、EXIF 和路径模板，会员导出预览可以比较画质与大小。保存、快速保存与自动保存使用不同路径规则。[标注](https://pixpin.cn/docs/annotation/basics) · [内置快捷键](https://pixpin.cn/docs/settings/built-in-shortcuts) · [保存设置](https://pixpin.cn/docs/settings/save)

## 贴图

| 类型 | PixPin 行为 |
| --- | --- |
| 图片 | 来自截图、剪贴板、文件或拖放。支持保留透明度、裁剪、旋转、翻转、标注、复制与导出。OCR 在图片上提供可选文字、划线、高亮和复制，3.6.6.0 增加双击选中一行。 |
| 文本 | 可保留或忽略格式，截取长文本片段、选择、复制、翻译，配置字体与显示方式。 |
| 文件与颜色 | 文件卡显示名称等信息，可以打开或复制文件，支持从文件管理器贴出选中项。颜色卡展示色块与多种值，可按格式复制。 |
| 公式 | 从 LaTeX 或公式识别结果生成置顶公式，可复制源码或渲染结果并导出。 |
| 动态图片 | 支持播放、暂停、倍速及保存动态内容。 |
| 实时窗口 | 仅 Windows。贴出来源窗口中的区域并实时更新，被遮挡也可继续观察。支持转换为静态贴图、旋转和翻转。来源最小化或关闭时存在单独的处理规则。 |

共享窗口行为包括移动、置顶、缩放、透明度、锁定、穿透、缩略模式、批量显隐和关闭。分组有名称、颜色、顺序，可以切换、迁移内容、清空或删除。关闭后可从贴图历史恢复，隐藏保留活跃状态，销毁会移除历史。可配置恢复顺序、上限、重复贴图和新启动时恢复。[贴图操作](https://pixpin.cn/docs/pin/basics) · [图片](https://pixpin.cn/docs/pin/image) · [文件](https://pixpin.cn/docs/pin/file) · [颜色](https://pixpin.cn/docs/pin/color) · [公式](https://pixpin.cn/docs/pin/formula) · [动态贴图](https://pixpin.cn/docs/pin/dynamic) · [实时窗口](https://pixpin.cn/docs/pin/window) · [分组](https://pixpin.cn/docs/pin/groups) · [历史](https://pixpin.cn/docs/pin/history)

## 识别与翻译

| 功能 | PixPin 行为 |
| --- | --- |
| 本地 OCR | 由截图或贴图触发，支持语言选择、对照原图校正、合并段落、处理空格与标点、复制局部或全文。贴图中的文字可以直接框选。 |
| 扫码 | 二维码可自动或手动检测，复制结果。二十余种工业条码属于会员功能。 |
| 翻译 | 会员联网功能，支持从图片识别后翻译，原位覆盖、选取复制、备用目标语言及朗读。 |
| 文档识别 | 会员功能。结构化还原标题、段落、图片、表格与公式，可以编辑 Markdown、对照预览，导出 DOCX、PDF、LaTeX、Markdown 等。手册对长截图直接用于文档识别的流程附有测试版标记。 |
| 表格识别 | 会员功能。对照图像编辑单元格，插删行列、合并拆分、排序、控制边框，复制全表或选区，导出 XLSX、CSV。 |
| 公式识别 | 会员功能。识别与校正多条 LaTeX 公式，选择定界符，复制为 LaTeX、MathML、AsciiMath、Typst、图像或可编辑 Word 公式，支持 Word、SVG 和多种图像导出。 |

[OCR](https://pixpin.cn/docs/recognition/text-recognition) · [会员权益](https://pixpin.cn/docs/getting-started/membership) · [翻译](https://pixpin.cn/features/translate/) · [文档](https://pixpin.cn/docs/recognition/document-recognition) · [表格](https://pixpin.cn/docs/recognition/table-recognition) · [公式](https://pixpin.cn/docs/recognition/formula-recognition)

## 长截图和录屏

- 长截图可以纵向或横向拼接。选区内可手动或自动滚动，实时显示拼接预览，支持暂停、重试和裁掉首尾。会员支持滚动方向改变时自动裁剪。结果可以复制、保存、贴图、编辑、OCR。官方称超长输出单边最高可到两百万像素，实现需要分块和有界资源管理。[长截图手册](https://pixpin.cn/docs/screenshot/long-screenshot)
- 普通录制可回放，快速录制直接输出 MP4。支持区域、帧率、延时、暂停恢复、重录、取消，系统音频和麦克风分别采集与调整。导出 MP4、GIF、WebP，动图还可用作贴图。会员支持裁切、剪辑、键鼠动画和按键叠层、进度显示、最多两路摄像头。[录屏手册](https://pixpin.cn/docs/screenshot/screen-recording) · [会员权益](https://pixpin.cn/docs/getting-started/membership)

## 设置、自动化与平台差异

- 支持全局快捷键及冲突提示、按上下文分别定制键盘和鼠标动作、忽略前台程序、配置入口排序、自定义脚本和命令行执行。脚本可取得活动窗口、鼠标屏幕、上次选区，指定区域并在截图后执行复制、保存或贴图。支持配置导入导出及会员云同步，部分机器本地路径不跨机器同步。[快捷键与动作](https://pixpin.cn/docs/settings/shortcuts-actions) · [全局鼠标](https://pixpin.cn/docs/settings/global-mouse) · [脚本](https://pixpin.cn/docs/settings/script) · [配置导入导出](https://pixpin.cn/docs/settings/export-import)
- 支持主题、颜色、视觉尺寸、字体、九种界面语言、音效、剪贴板选项、OCR 与翻译偏好、开机启动、自动更新、代理、使用分析设置、账号和会员管理，以及恢复默认设置。[外观](https://pixpin.cn/docs/settings/appearance) · [系统](https://pixpin.cn/docs/settings/system) · [账号](https://pixpin.cn/docs/settings/account)
- macOS 安装包同时包含 Intel 和 Apple Silicon 应用，官方最低支持 10.15。截图、系统音频、辅助功能、麦克风、摄像头分别涉及系统权限。Windows 还有实时窗口贴图、Explorer 右键菜单、管理员启动、托盘点击动作及全屏游戏快捷键选项。按键组合和部分设置的默认值也会随平台变化。[安装与权限](https://pixpin.cn/docs/getting-started/install-and-update) · [快捷键与动作](https://pixpin.cn/docs/settings/shortcuts-actions) · [系统](https://pixpin.cn/docs/settings/system)

## 本机分发物证据

`/Applications/PixPin.app/Contents/Info.plist` 显示 3.6.6.0，构建 9090。`rea inspect-macho /Applications/PixPin.app --json` 识别到 arm64 与 x86_64 两个切片。主程序直接链接 Qt 5.15.13 系列、AVFoundation、CoreMedia、图像处理和 OCR 相关库，并弱链接 ScreenCaptureKit。应用资源目录可见语言包与 ONNX 等模型文件。包级扫描识别到 286 个条目，探测了 77 个文件。链接和文件证明分发物包含这些组件，具体功能、权限下的回退和各系统的执行路径需要运行验证。

- REA Mach-O 证据 `ev_d2821a23a906daf833245c50ba8992d96b0e375b45ea7968fe583000abb18dd1`，主程序 SHA-256 `d30e1cdd8aac83436e0bbb318a78859cf18bd03a62763a899555b2a42bd17698`。

## Rshot 的实现顺序

| 顺序 | 交付方向 | 验收重点 |
| --- | --- | --- |
| 1 | macOS 14+，GPUI 自有界面，ScreenCaptureKit 截取当前窗口所在的单个显示器，矩形拖选，复制、PNG、图片和文字贴图 | 退出不输出，Retina 裁切保持原始像素，保存取消后返回选区，贴图可拖动、滚轮缩放及关闭。 |
| 2 | 菜单栏、全局键、窗口探测、逐像素编辑选区、跨屏、历史、图片标注、本地 OCR 与扫码 | 系统权限拒绝、快捷键冲突、混合 DPI、窗口遮挡、撤销重做、识别坐标与图像缩放保持一致。macOS OCR 可先评估 Vision。 |
| 3 | 贴图历史与分组、穿透和透明度、横纵长图、录屏和导出 | 匹配失败时明确提示，重复内容与固定页眉场景可处理。长图与录屏不得随时长无限累积未压缩帧。 |
| 4 | Windows 捕获与窗口集成、丰富输出、文档、表格、公式及翻译、自定义动作与设置 | Windows 的 WGC/DXGI、UI Automation 和常驻入口单独验证。结构化识别使用独立提供方和测试语料，联网翻译需要明确的网络行为与数据范围。 |

GPUI 负责显示、输入、绘制与窗口组织。操作系统负责截图、权限、窗口层级、全局输入和录屏。图像编辑与导出使用独立的像素数据，显示器逻辑坐标只在交互层使用；添加 Windows 后可复用裁切、历史和标注模型。
