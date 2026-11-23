use std::{cell::RefCell, collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context as _, Result, anyhow, ensure};
use gpui::{prelude::FluentBuilder, *};
use image::{Frame, ImageEncoder, RgbaImage, codecs::png::PngEncoder, imageops::crop_imm};
use objc2::{msg_send, runtime::AnyObject};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::{capture, macos, stitch::Axis};

mod editor;
mod long_shot;

const BACKGROUND: u32 = 0x101823;
const PANEL: u32 = 0x1e2c3b;
const FOREGROUND: u32 = 0xf2f6fa;
const MUTED: u32 = 0xa7b9cc;
const ACCENT: u32 = 0x75ebc5;

pub fn run() {
    Application::new().run(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(420.), px(250.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some("Rshot".into()),
                ..Default::default()
            }),
            is_resizable: false,
            ..Default::default()
        };
        if let Err(error) = cx.open_window(options, |_, cx| cx.new(|_| Launcher::default())) {
            eprintln!("Rshot: {error:#}");
            cx.quit();
            return;
        }
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.activate(true);
    });
}

type LabelKey = (SharedString, u32, u32, u32);

thread_local! {
    static LABEL_TEXTURES: RefCell<HashMap<LabelKey, Arc<RenderImage>>> = RefCell::default();
}

fn ui_label(text: impl Into<SharedString>, color: u32, size: u32, max_width: u32) -> Div {
    let text = text.into();
    if text.is_empty() {
        return div();
    }
    // AppKit shapes the text; GPUI paints it through the color-image atlas.
    const SCALE: u32 = 2;
    let texture = LABEL_TEXTURES.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = (text.clone(), color, size, max_width);
        if let Some(texture) = cache.get(&key) {
            return Ok(texture.clone());
        }
        let image = crate::annotation::interface_text(
            &text,
            f64::from(size * SCALE),
            max_width * SCALE,
            color,
        )?;
        let texture = render_image(image);
        if cache.len() >= 128 {
            cache.clear();
        }
        cache.insert(key, texture.clone());
        Ok::<_, anyhow::Error>(texture)
    });
    match texture {
        Ok(texture) => {
            let size = texture.size(0);
            div().flex_shrink_0().child(
                img(texture)
                    .w(px(size.width.0 as f32 / SCALE as f32))
                    .h(px(size.height.0 as f32 / SCALE as f32)),
            )
        }
        Err(_) => div()
            .text_color(rgb(color))
            .text_size(px(size as f32))
            .child(text),
    }
}

fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(36.))
        .px_4()
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(if primary { ACCENT } else { PANEL }))
        .flex_shrink_0()
        .hover(|style| style.opacity(0.84))
        .child(ui_label(
            label,
            if primary { BACKGROUND } else { FOREGROUND },
            14,
            180,
        ))
}

#[derive(Default)]
struct Launcher {
    busy: bool,
    overlay: Option<WindowHandle<Selection>>,
    status: SharedString,
}

impl Launcher {
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if let Some(handle) = self.overlay.take()
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            self.overlay = Some(handle);
            return;
        }
        let Some(display) = window.display(cx).or_else(|| cx.primary_display()) else {
            self.status = "找不到可用显示器".into();
            cx.notify();
            return;
        };
        if let Err(error) = capture::ready() {
            self.status = format!("{error:#}").into();
            cx.notify();
            return;
        }
        let display_id = display.id();
        let bounds = display.bounds();
        let scale = window.scale_factor();
        self.busy = true;
        self.status = "正在读取屏幕…".into();
        cx.notify();

        let task = cx.background_spawn(async move {
            let targets = macos::capture_targets(display_id.into());
            capture::screen(display_id.into(), scale).map(|image| (image, targets))
        });
        cx.spawn(async move |launcher, cx| {
            let captured = task.await;
            let _ = launcher.update(cx, |launcher, cx| {
                launcher.busy = false;
                let result = captured.and_then(|(image, targets)| {
                    let handle = cx.open_window(
                        WindowOptions {
                            window_bounds: Some(WindowBounds::Windowed(bounds)),
                            display_id: Some(display_id),
                            titlebar: None,
                            kind: WindowKind::PopUp,
                            is_movable: false,
                            is_resizable: false,
                            is_minimizable: false,
                            ..Default::default()
                        },
                        |window, cx| {
                            cx.new(|cx| {
                                Selection::new(image, display_id, bounds.size, window, cx)
                                    .with_targets(targets)
                            })
                        },
                    )?;
                    handle.update(cx, |selection, window, cx| {
                        if let Err(error) = macos::fit_capture_overlay(window, display_id.into()) {
                            close_window(window);
                            return Err(error);
                        }
                        selection.hovered =
                            Some(selection.target_at(window.mouse_position(), bounds.size));
                        cx.notify();
                        Ok(())
                    })??;
                    Ok(handle)
                });
                match result {
                    Ok(handle) => {
                        launcher.overlay = Some(handle);
                        launcher.status = "".into();
                    }
                    Err(error) => launcher.status = format!("{error:#}").into(),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pin_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let result = (|| -> Result<()> {
            let item = cx.read_from_clipboard().context("剪贴板没有图片或文字")?;
            let content = if let Some(image) = item.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image),
                _ => None,
            }) {
                let image = image::load_from_memory(&image.bytes).context("无法读取剪贴板图片")?;
                PinContent::Image(image.to_rgba8())
            } else {
                let text = item
                    .text()
                    .filter(|text| !text.is_empty())
                    .context("剪贴板没有图片或文字")?;
                PinContent::Text(text.into())
            };
            let display = window
                .display(cx)
                .or_else(|| cx.primary_display())
                .context("找不到可用显示器")?;
            open_pin(
                content,
                display.id(),
                display.bounds().size,
                None,
                window.scale_factor(),
                cx,
            )
        })();
        self.status = match result {
            Ok(()) => "".into(),
            Err(error) => format!("{error:#}").into(),
        };
        cx.notify();
    }
}

impl Render for Launcher {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .justify_center()
            .gap_4()
            .px_6()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui_label("截图与贴图", FOREGROUND, 20, 340)),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        button(
                            "capture",
                            if self.busy {
                                "正在截图…"
                            } else {
                                "区域截图"
                            },
                            true,
                        )
                        .on_click(cx.listener(|this, _, window, cx| this.start(window, cx))),
                    )
                    .child(button("clipboard", "贴出剪贴板", false).on_click(
                        cx.listener(|this, _, window, cx| this.pin_clipboard(window, cx)),
                    )),
            )
            .when(!self.status.is_empty(), |root| {
                root.child(div().text_sm().text_color(rgb(MUTED)).child(ui_label(
                    self.status.clone(),
                    MUTED,
                    13,
                    350,
                )))
            })
    }
}

fn render_image(mut image: RgbaImage) -> Arc<RenderImage> {
    // GPUI uploads the frame to a BGRA texture.
    for pixel in image.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    Arc::new(RenderImage::new([Frame::new(image)]))
}

pub(crate) fn png_bytes(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(png)
}

fn crop_bounds(rect: Bounds<Pixels>, viewport: Size<Pixels>, image: &RgbaImage) -> [u32; 4] {
    let sx = image.width() as f32 / f32::from(viewport.width);
    let sy = image.height() as f32 / f32::from(viewport.height);
    let right = rect.origin.x + rect.size.width;
    let bottom = rect.origin.y + rect.size.height;
    let x0 = (f32::from(rect.origin.x) * sx).floor() as u32;
    let y0 = (f32::from(rect.origin.y) * sy).floor() as u32;
    let x1 = ((f32::from(right) * sx).ceil() as u32).min(image.width());
    let y1 = ((f32::from(bottom) * sy).ceil() as u32).min(image.height());
    let x0 = x0.min(x1);
    let y0 = y0.min(y1);
    [x0, y0, x1 - x0, y1 - y0]
}

fn selection_rect(a: Point<Pixels>, b: Point<Pixels>, view: Size<Pixels>) -> Bounds<Pixels> {
    let a = a.clamp(&point(px(0.), px(0.)), &point(view.width, view.height));
    let b = b.clamp(&point(px(0.), px(0.)), &point(view.width, view.height));
    let origin = point(a.x.min(b.x), a.y.min(b.y));
    Bounds {
        origin,
        size: size(a.x.max(b.x) - origin.x, a.y.max(b.y) - origin.y),
    }
}

fn place(rect: Bounds<Pixels>) -> Div {
    div()
        .absolute()
        .left(rect.origin.x)
        .top(rect.origin.y)
        .w(rect.size.width)
        .h(rect.size.height)
}

fn shade(x: Pixels, y: Pixels, w: Pixels, h: Pixels) -> Div {
    place(Bounds {
        origin: point(x, y),
        size: size(w, h),
    })
    .bg(rgba(0x0b12299f))
}

#[derive(Clone, Copy)]
enum Output {
    Copy,
    Save,
    Pin,
    Edit,
    Long(Axis),
}

struct Selection {
    image: RgbaImage,
    texture: Arc<RenderImage>,
    display_id: DisplayId,
    screen_size: Size<Pixels>,
    scale: f32,
    focus: FocusHandle,
    drag: Option<(Point<Pixels>, Point<Pixels>)>,
    menu_bar: Option<Bounds<Pixels>>,
    windows: Vec<Bounds<Pixels>>,
    hovered: Option<Bounds<Pixels>>,
    selection: Option<Bounds<Pixels>>,
    saving: bool,
    status: SharedString,
}

impl Selection {
    fn new(
        image: RgbaImage,
        display_id: DisplayId,
        screen_size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window);
        Self {
            texture: render_image(image.clone()),
            image,
            display_id,
            screen_size,
            scale: window.scale_factor(),
            focus,
            drag: None,
            menu_bar: None,
            windows: Vec::new(),
            hovered: None,
            selection: None,
            saving: false,
            status: "".into(),
        }
    }

    fn with_targets(mut self, targets: macos::CaptureTargets) -> Self {
        let convert = |rect: objc2_core_foundation::CGRect| Bounds {
            origin: point(px(rect.origin.x as f32), px(rect.origin.y as f32)),
            size: size(px(rect.size.width as f32), px(rect.size.height as f32)),
        };
        self.menu_bar = targets.menu_bar.map(convert);
        self.windows = targets.windows.into_iter().map(convert).collect();
        self
    }

    fn target_at(&self, position: Point<Pixels>, view: Size<Pixels>) -> Bounds<Pixels> {
        capture_target_at(self.menu_bar, &self.windows, position, view)
    }

    fn finish(&mut self, output: Output, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving || self.drag.is_some() {
            return;
        }
        let view = window.viewport_size();
        let rect = self.selection.or(self.hovered).unwrap_or(Bounds {
            origin: point(px(0.), px(0.)),
            size: view,
        });
        let [x, y, width, height] = crop_bounds(rect, view, &self.image);
        if width == 0 || height == 0 {
            return;
        }
        let cropped = crop_imm(&self.image, x, y, width, height).to_image();
        match output {
            Output::Copy => match png_bytes(&cropped) {
                Ok(bytes) => {
                    let image = Image::from_bytes(ImageFormat::Png, bytes);
                    cx.write_to_clipboard(ClipboardItem::new_image(&image));
                    close_window(window);
                }
                Err(error) => self.report(error, cx),
            },
            Output::Pin => match open_pin(
                PinContent::Image(cropped),
                self.display_id,
                self.screen_size,
                Some(rect.origin),
                self.scale,
                cx,
            ) {
                Ok(()) => close_window(window),
                Err(error) => self.report(error, cx),
            },
            Output::Save => self.save(cropped, window, cx),
            Output::Edit => {
                match editor::open(cropped, self.scale, self.display_id, self.screen_size, cx) {
                    Ok(()) => close_window(window),
                    Err(error) => self.report(error, cx),
                }
            }
            Output::Long(axis) => {
                let shot = long_shot::LongShot::new(
                    axis,
                    cropped,
                    self.display_id,
                    self.scale,
                    [x, y, width, height],
                    self.image.dimensions(),
                );
                match shot.and_then(|shot| shot.open(rect, self.screen_size, cx)) {
                    Ok(()) => close_window(window),
                    Err(error) => self.report(error, cx),
                }
            }
        }
    }

    fn report(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.status = format!("{error:#}").into();
        cx.notify();
    }

    fn save(&mut self, image: RgbaImage, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(error) = hide_native_window(window) {
            self.report(error, cx);
            return;
        }
        self.saving = true;
        self.status = "".into();
        cx.notify();
        let directory = std::env::home_dir().unwrap_or_else(|| Path::new(".").into());
        let dialog = cx.prompt_for_new_path(&directory, Some("Rshot.png"));
        cx.spawn_in(window, async move |this, cx| {
            let result = match dialog.await {
                Ok(Ok(Some(path))) => {
                    let task = cx.background_spawn(async move {
                        image
                            .save_with_format(&path, image::ImageFormat::Png)
                            .with_context(|| format!("无法保存 PNG 到 {}", path.display()))
                    });
                    task.await.map(|()| true)
                }
                Ok(Ok(None)) => Ok(false),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow!(error)),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.saving = false;
                match result {
                    Ok(true) => window.remove_window(),
                    Ok(false) => {
                        window.activate_window();
                        this.focus.focus(window);
                        cx.notify();
                    }
                    Err(error) => {
                        window.activate_window();
                        this.focus.focus(window);
                        this.report(error, cx);
                    }
                }
            });
        })
        .detach();
    }
}

fn close_window(window: &mut Window) {
    let _ = hide_native_window(window);
    window.remove_window();
}

fn hide_native_window(window: &Window) -> Result<()> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("无法取得截图窗口句柄 ({error:?})"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(anyhow!("无法找到截图窗口"));
    };
    // GPUI owns the view and its AppKit window. The event runs on the main thread.
    unsafe {
        let view = handle.ns_view.cast::<AnyObject>().as_ref();
        let native: *mut AnyObject = msg_send![view, window];
        let _: () = msg_send![native, orderOut: std::ptr::null::<AnyObject>()];
    }
    Ok(())
}

fn capture_target_at(
    menu_bar: Option<Bounds<Pixels>>,
    windows: &[Bounds<Pixels>],
    position: Point<Pixels>,
    view: Size<Pixels>,
) -> Bounds<Pixels> {
    menu_bar
        .into_iter()
        .chain(windows.iter().copied())
        .find(|bounds| {
            position.x >= bounds.origin.x
                && position.x < bounds.origin.x + bounds.size.width
                && position.y >= bounds.origin.y
                && position.y < bounds.origin.y + bounds.size.height
        })
        .unwrap_or(Bounds {
            origin: point(px(0.), px(0.)),
            size: view,
        })
}

impl Render for Selection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = window.viewport_size();
        let rect = self
            .drag
            .map(|(start, end)| selection_rect(start, end, view))
            .or(self.selection)
            .or(self.hovered);
        let mut root = div()
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(rgb(BACKGROUND))
            .cursor_crosshair()
            .child(
                img(self.texture.clone())
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Fill),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match (
                    event.keystroke.key.as_str(),
                    event.keystroke.modifiers.platform,
                ) {
                    ("escape", _) if !this.saving => close_window(window),
                    ("c", true) | ("enter", false) => this.finish(Output::Copy, window, cx),
                    ("s", true) => this.finish(Output::Save, window, cx),
                    ("p", true) => this.finish(Output::Pin, window, cx),
                    ("e", false) | ("e", true) => this.finish(Output::Edit, window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    if this.saving {
                        return;
                    }
                    this.drag = Some((event.position, event.position));
                    this.selection = None;
                    this.status = "".into();
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                if let Some((_, end)) = &mut this.drag {
                    *end = event.position;
                    cx.notify();
                } else if this.selection.is_none() {
                    let hovered = Some(this.target_at(event.position, window.viewport_size()));
                    if this.hovered != hovered {
                        this.hovered = hovered;
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    if let Some((start, _)) = this.drag.take() {
                        let rect = selection_rect(start, event.position, window.viewport_size());
                        let movement = ((event.position.x - start.x).abs())
                            .max((event.position.y - start.y).abs());
                        this.selection = if movement < px(3.) {
                            Some(this.target_at(event.position, window.viewport_size()))
                        } else if rect.size.width >= px(1.) && rect.size.height >= px(1.) {
                            Some(rect)
                        } else {
                            None
                        };
                        this.hovered = None;
                        cx.notify();
                    }
                }),
            );

        if let Some(rect) = rect {
            let Point { x, y } = rect.origin;
            let Size { width, height } = rect.size;
            let bottom = y + height;
            let right = x + width;
            root = root
                .child(shade(px(0.), px(0.), view.width, y))
                .child(shade(px(0.), bottom, view.width, view.height - bottom))
                .child(shade(px(0.), y, x, height))
                .child(shade(right, y, view.width - right, height))
                .child(place(rect).border_1().border_color(rgb(ACCENT)));
        } else {
            root = root.child(shade(px(0.), px(0.), view.width, view.height));
        }

        let label: SharedString = if let Some(rect) = rect {
            let [_, _, w, h] = crop_bounds(rect, view, &self.image);
            format!("{w} × {h} px").into()
        } else {
            "点击选取窗口或拖动框选 · Esc 退出".into()
        };
        let mut controls = div()
            .id("capture-actions")
            .absolute()
            .top(
                self.menu_bar
                    .map_or(px(30.), |bar| bar.size.height + px(8.)),
            )
            .left(px(20.))
            .flex()
            .items_center()
            .gap_2()
            .p_2()
            .rounded_lg()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .text_sm()
            .child(div().px_2().child(ui_label(label, FOREGROUND, 13, 330)));
        if self.selection.is_some() {
            controls = controls
                .cursor_default()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(button("edit", "编辑", true).on_click(
                    cx.listener(|this, _, window, cx| this.finish(Output::Edit, window, cx)),
                ))
                .child(button("copy", "复制", false).on_click(
                    cx.listener(|this, _, window, cx| this.finish(Output::Copy, window, cx)),
                ))
                .child(
                    button(
                        "save",
                        if self.saving {
                            "保存中…"
                        } else {
                            "保存 PNG"
                        },
                        false,
                    )
                    .on_click(
                        cx.listener(|this, _, window, cx| this.finish(Output::Save, window, cx)),
                    ),
                )
                .child(button("pin", "置顶贴图", false).on_click(
                    cx.listener(|this, _, window, cx| this.finish(Output::Pin, window, cx)),
                ))
                .child(
                    button("long-vertical", "纵向长截图", false).on_click(cx.listener(
                        |this, _, window, cx| this.finish(Output::Long(Axis::Vertical), window, cx),
                    )),
                )
                .child(
                    button("long-horizontal", "横向长截图", false).on_click(cx.listener(
                        |this, _, window, cx| {
                            this.finish(Output::Long(Axis::Horizontal), window, cx)
                        },
                    )),
                );
        }
        root.child(controls).when(!self.status.is_empty(), |root| {
            root.child(
                div()
                    .absolute()
                    .bottom_4()
                    .left_5()
                    .px_4()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(BACKGROUND))
                    .text_color(rgb(FOREGROUND))
                    .child(ui_label(self.status.clone(), FOREGROUND, 14, 420)),
            )
        })
    }
}

enum PinContent {
    Image(RgbaImage),
    Text(SharedString),
}

enum PinView {
    Image(Arc<RenderImage>),
    Text(SharedString),
}

struct Pin {
    view: PinView,
    focus: FocusHandle,
    natural: Size<Pixels>,
    zoom: f32,
    max_zoom: f32,
}

fn open_pin(
    content: PinContent,
    display: DisplayId,
    screen: Size<Pixels>,
    origin: Option<Point<Pixels>>,
    scale: f32,
    cx: &mut App,
) -> Result<()> {
    let (view, natural) = match content {
        PinContent::Image(image) => {
            let dimensions = size(
                px(image.width() as f32 / scale),
                px(image.height() as f32 / scale),
            );
            (PinView::Image(render_image(image)), dimensions)
        }
        PinContent::Text(text) => (PinView::Text(text), size(px(340.), px(180.))),
    };
    let fit = (f32::from(screen.width) * 0.85 / f32::from(natural.width))
        .min((f32::from(screen.height) * 0.85 - 35.) / f32::from(natural.height));
    ensure!(fit > 0., "显示器空间不足");
    let zoom = fit.min(1.);
    let width = (natural.width * zoom).max(px(140.)).min(screen.width);
    let height = (natural.height * zoom)
        .max(px(48.))
        .min(screen.height - px(35.));
    let max_origin = point(
        (screen.width - width).max(px(0.)),
        (screen.height - height - px(35.)).max(px(24.)),
    );
    let centered = point((screen.width - width) / 2., (screen.height - height) / 2.);
    let origin = origin
        .unwrap_or(centered)
        .clamp(&point(px(0.), px(24.)), &max_origin);
    cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Rshot · 贴图".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin,
                size: size(width, height),
            })),
            display_id: Some(display),
            kind: WindowKind::PopUp,
            is_minimizable: false,
            is_resizable: false,
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window);
                Pin {
                    view,
                    focus,
                    natural,
                    zoom,
                    max_zoom: fit,
                }
            })
        },
    )?;
    Ok(())
}

impl Render for Pin {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("pin-content")
            .track_focus(&self.focus)
            .size_full()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                if key.key == "escape" || (key.key == "w" && key.modifiers.platform) {
                    close_window(window);
                    cx.stop_propagation();
                }
            }));
        match &self.view {
            PinView::Image(texture) => root
                .child(
                    img(texture.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, window, _| {
                    let delta = event.delta.pixel_delta(px(16.)).y;
                    if delta == px(0.) {
                        return;
                    }
                    let step = if delta > px(0.) { 1.08 } else { 1. / 1.08 };
                    this.zoom = (this.zoom * step).clamp(this.max_zoom.min(0.1), this.max_zoom);
                    window.resize(size(
                        (this.natural.width * this.zoom).max(px(140.)),
                        (this.natural.height * this.zoom).max(px(48.)),
                    ));
                }))
                .into_any_element(),
            PinView::Text(text) => root
                .overflow_y_scroll()
                .p_4()
                .child(text.clone())
                .into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, point, px, size};
    use image::RgbaImage;

    use super::{capture_target_at, crop_bounds, png_bytes, selection_rect};

    #[test]
    fn capture_prefers_menu_and_frontmost_window_then_falls_back_to_full_screen() {
        let view = size(px(100.), px(80.));
        let screen = Bounds {
            origin: point(px(0.), px(0.)),
            size: view,
        };
        let menu = Bounds {
            size: size(px(100.), px(5.)),
            ..screen
        };
        let front = Bounds {
            origin: point(px(10.), px(4.)),
            size: size(px(30.), px(30.)),
        };
        let back = Bounds {
            origin: point(px(5.), px(4.)),
            size: size(px(60.), px(45.)),
        };
        let windows = &[front, back];
        assert_eq!(
            capture_target_at(Some(menu), windows, point(px(20.), px(4.)), view),
            menu
        );
        assert_eq!(
            capture_target_at(Some(menu), windows, point(px(20.), px(5.)), view),
            front
        );
        assert_eq!(
            capture_target_at(None, windows, point(px(90.), px(50.)), view),
            screen
        );
        assert_eq!(
            crop_bounds(menu, view, &RgbaImage::new(200, 160)),
            [0, 0, 200, 10]
        );
    }

    #[test]
    fn reversed_drag_is_clipped_and_converted_to_retina_pixels() {
        let view = size(px(100.), px(80.));
        let rect = selection_rect(point(px(125.), px(90.)), point(px(4.25), px(7.25)), view);
        assert_eq!(
            crop_bounds(rect, view, &RgbaImage::new(200, 160)),
            [8, 14, 192, 146]
        );
    }

    #[test]
    fn png_preserves_color_and_alpha() {
        let image = RgbaImage::from_raw(1, 1, vec![250, 22, 100, 127]).unwrap();
        assert_eq!(
            image::load_from_memory(&png_bytes(&image).unwrap())
                .unwrap()
                .to_rgba8(),
            image
        );
    }
}
