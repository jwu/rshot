# PixPin 3.6.6.0 macOS 实现原理

分析对象是本机 `/Applications/PixPin.app`，版本 3.6.6.0，构建 9090。主程序是同时包含 arm64 和 x86_64 的 Mach-O。本次检查 arm64，主程序文件 SHA-256 为 `d30e1cdd8aac83436e0bbb318a78859cf18bd03a62763a899555b2a42bd17698`。用 REA `inspect-macho` 查看模块、导入和导出，再用 `inspect-with-objdump` 追踪实际指令中的调用。Apple LLVM 的符号选项通过临时适配器传给 REA。本机 REA 没有可用的伪代码提供方，本报告依据静态符号、指令和常量，运行时选路及 Windows 实现仍需独立验证。

## 截图和多屏选区

- 主程序使用 C++ 和 Qt 5.15.13。`ScreenShotView::prepareScreenShotBgLayer` 调用 `PixScreenManager::screenList`，建立截图背景层并准备背景图。`ScreenShotWindow::refreshScreenShotMacos` 取得 `wholeRect` 和显示器列表，创建 `AuxiliaryShotWindow` 并刷新截图视图。背景像素、显示器布局和选区窗口有各自的职责。
- `libPixScreenManager` 有两条静态截图路径。Qt 路径在 `0x9564` 调用 `QScreen::grabWindow`。CoreGraphics 路径在 `0xc9e0` 用 `dlsym` 解析 `CGDisplayCreateImageForRect`，因此 Mach-O 的导入表里没有这项函数。库还读取 `CGImage` 的位深、行跨度、像素数据与显示器色彩空间。显示器枚举失败时日志明确记录从 CoreGraphics 回退到 Qt。截图后端在各版本或配置下的优先级尚未确认。
- `PixScreenManager::grabScreen` 对显示器矩形做相交和包含测试，跨屏时用 `QPainter::drawPixmap` 合成显示器交集，还可以单独调用 `PixCursorPixmap::GetCursor`。逻辑坐标、源图采样和光标合成是不同步骤。Rshot 当前使用 `SCScreenshotManager`，可以保留这条原生接口并复用这种数据划分。

## 窗口和 UI 元素定位

- `libUiRegionDetector` 的 `DirectGetRect` 在 `0x3960` 创建系统级 Accessibility 对象，在 `0x3980` 用 `AXUIElementCopyElementAtPosition` 查询鼠标位置的元素。同库导入 `AXUIElementCopyAttributeValue`，并暴露父、子元素矩形查询、仅窗口模式及缓存重置接口。这说明命中区域的主要输入是辅助功能对象及其几何信息。
- `libPixWindowNotify` 在 `0x33cc` 调用 `CGWindowListCopyWindowInfo`，还引用窗口编号、层级和所属进程字段。Rshot 可以用窗口列表实现窗口级候选，再把 Accessibility 查询用于细粒度选区。当前证据没有覆盖 PixPin 对各类应用的全部降级规则。

## 贴图、历史与全局输入

- 贴图由 `PinWindowBase` 及不同内容窗口管理。`PinWindowBase::setMousePenetration` 在 `0x1000f42d4` 调用 `libPixSystemUtils` 的 `SetWindowIgnoreMouse(QWidget*, bool)`。该函数从 `QWidget::winId()` 取得原生视图并访问窗口，同库包含 AppKit 的 `setIgnoresMouseEvents:`。置顶另有 `PinWindowTopmostState` 和 `refreshTopmostState`。
- `PinDataQuery` 通过 `DataQuery` 初始化名为 `PinDatabase` 的记录库，主程序存在 `QSQLITE` 驱动字符串。可见的方法包括按组查询、查询最近关闭、移动贴图到组、更新与删除，另有 `PinWindowData::save/load` 及关联文件接口。证据支持 QtSql/SQLite 管理贴图记录，图像资产和恢复规则的具体格式还未追踪。
- `libqxtglobalshortcut` 在 `0x2920` 调用 Carbon `RegisterEventHotKey`。`libPixKeyMouse` 在 `0x8d78` 调用 `CGEventTapCreate`，并将事件源接到运行循环。这给出了快捷键注册和键鼠事件监测两条路径。主程序还导入 `QJSEngine::evaluate`、`newQObject` 和 `globalObject`，支持用 Qt 的 JavaScript 引擎绑定原生动作。

## 长截图

主程序中同一处理路径先调用 `SimulateMouseScroll` 和 `PixScreenManager::grabScreen`（`0x1001b3b1c`、`0x1001b3b34`），再把帧交给 `PixStitching::tryAddImage`（`0x1001b3bf0`）。`tryAddImage` 调用 `DetectDisplacement`，后者调用 `findDiffRect`、`calcOffsetScoreMat`、`clearOffsetMaskByScore`。结合函数中的调用，可以还原以下操作顺序。

1. 比较两帧，用 OpenCV `absdiff` 与阈值生成差异信息，再用行列归约定位变化区域。
2. 在匹配路径中使用缩放和掩码，并行计算候选偏移的分数，再按得分清理候选。方向由 `SlideDirection` 传入。
3. `tryAddImage` 根据位移裁切帧，把新露出的内容交给 `PixLongImage::contactImage`。该函数可向首端或末端扩展，检查缺口并按 tile 管理内容。`getLogicalImageSlice` 暴露按逻辑范围读取的能力。

对 Rshot，直接可用的设计是把「采帧和滚动」「位移估计」「分块追加和预览」分开，先用固定页眉、重复条纹、低纹理、反向滚动准备测试图像。当前证据没有还原评分公式、候选阈值或各失败提示的触发条件。

## OCR、扫码和录制

| 功能 | 原生实现证据 |
| --- | --- |
| OCR | `libPixOCR2` 暴露 `PixOcrDetSplit`、`PixOcrRec`、`PixOcrLayout`，以及检测、识别、版面完成的独立回调。它导入 `PixModelRunner::inference`，后者链接 ONNX Runtime 的 `OrtGetApiBase` 并有 `Ort::Session` 缓存。资源中有 `paragraph_recognition.onnx`。主程序的 `OcrTextLayerItem`、`OcrTextLineItem` 提供按几何区域选中文本的接口。支持分阶段识别与可选文字层的结构，检测模型、分词和置信度细节尚未确认。 |
| 扫码 | `libPixVision` 包含 ZXing 的 `ReadBarcode`、`ReadBarcodes` 和二值化组件。它提供了可用的扫码实现，调用方式和各码制的产品规则仍需独立追踪。 |
| 快速录制 | 主程序 `QuickScreenRecorder` 创建 `PixScreenStreamer`。`onFrameTimeout` 在 `0x100009e38` 调用 `captureTo(QImage&)`，确认有以定时器驱动的取帧路径。 |
| 多轨录制 | `libPixMovie` 暴露普通多轨录制、暂停、导出和剪切。证据中 `SCKAudioCaptureDelegate` 与 `SCContentFilter`、`SCStreamConfiguration`、`SCStream` 同处一条系统音频采集路径，旁边还有 `AVAssetWriter` 和添加音频输入的日志。主程序另外有 AVFoundation 摄像头入口。`libPixMovie` 导入多轨混合与 H.264、GIF 和 WebP 编码接口。`libPixAVCodec` 还导入 VideoToolbox 的 `VTCompressionSessionEncodeFrame`。各编码设置及普通录制的视频取帧方式尚未完整核对。 |

## REA 证据索引

在同一文件上，Mach-O 查看和符号反汇编分别有自己的证据 ID。地址均是所选 arm64 Mach-O 的静态虚拟地址。

| 范围 | REA 证据 ID |
| --- | --- |
| 主程序 Mach-O | `ev_d2821a23a906daf833245c50ba8992d96b0e375b45ea7968fe583000abb18dd1` |
| 截图背景层 | `ev_3f7e3640ff746fefd4b4c9a7e27ad7c0beab51c9c1b9db3e00f047634eac5bab` |
| macOS 多屏截图视图 | `ev_42421560d9bb421c8a485a2e1c1c8e00a4226c989556376df77fce2e2f3e9698` |
| `libPixScreenManager` 反汇编 | `ev_f8c97ad741c8a9a14de77a4ec2cf955464fe4281198e1c091838990def625fc2` |
| `libUiRegionDetector` 反汇编 | `ev_9be94b68f266c443d01984ae95126f141db7cc4aebab548ed13e1029f3e4ec0b` |
| `libPixWindowNotify` 反汇编 | `ev_88fc2e2460f975ef3818175f255fc50ad527a788871e0f34b41e54481f23071e` |
| 贴图穿透调用 | `ev_50595025693c9c0b31a01ce478c0db3e240531dbc2953e0f8dc66428f44e616e` |
| 原生窗口辅助函数 | `ev_97787a57091209b3a0e07533b832ac9eb03fb818605c76bc77ccab16d3dcb073` |
| 贴图数据库构造 | `ev_f77cc975f86717cf57d30c4bd318529a407059038d305f55a291a1177a937c4a` |
| 长截图加帧与位移 | `ev_fb77a1e642d721504bc2e28c7c195b60ce44845d02f5603d2e402aae581c1291`、`ev_3afa26f9a912a971a9b9476025fd6b203e0bc5c3812ee246ed8d33941b70274e` |
| 长截图评分与分块 | `ev_37e850792219e164eb13943f03d2a3bf54e676ad453148a323a800da2189902e`、`ev_25bf2bdc1036d88b142cd4570c4bd132acda3336db6b404326c37fc556d4888d` |
| OCR、模型运行器、扫码 | `ev_9acd6f1cce544b61ed67ba073ce8d4cf1e588aaa28e00d80fd5a86db6836f56d`、`ev_17e57f81ad6a1327ddda3de98ad47f0638df5c38fe03934b99965bedb8d0a78b`、`ev_03d4b6dda8bf52f384a47574338cd5603c03a580f84fee5dc9d94fb1458aec92` |
| 快速录制取帧 | `ev_052be3eb2690cdb4cf5f66d9af4108d89b34fd371bc87219c8dc116750c94ae2` |
| `libPixMovie` 反汇编 | `ev_87e326f55ad0044c55a413532995c036972d7b926c9d0849599ca253424adf2d` |
| 编码库 Mach-O | `ev_65a26256e58258c5fb895d7a9dea59c3dffa0b5298556d15566f3b2df09d2d61` |
| 全局键盘与鼠标 | `ev_9eee1fbbb36e75ef132a74762b5ce6191efe5c9123d9d6784f34c175cdf5f587`、`ev_f6f9787f34bb330d1995db555bc0ffc03bb8f74a251ce83a8ce71000ad145f33` |
