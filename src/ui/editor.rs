use std::{cell::Cell, sync::Arc};

use anyhow::{Context as _, Result, anyhow};
use block2::RcBlock;
use futures_channel::oneshot;
use gpui::*;
use image::RgbaImage;
use objc2::{MainThreadMarker, MainThreadOnly, msg_send, rc::Retained};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSTextField, NSWindow};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::{
    BACKGROUND, FOREGROUND, MUTED, PANEL, button, close_window, place, png_bytes, render_image,
    selection_rect,
};
use crate::annotation::{self, Document, Mark};

const TOOLBAR: f32 = 58.;
const FOOTER: f32 = 38.;
const MARGIN: f32 = 16.;
const GUIDE_COLOR: u32 = 0xff5566;

pub(super) fn open(
    image: RgbaImage,
    scale: f32,
    display: DisplayId,
    screen: Size<Pixels>,
    cx: &mut App,
) -> Result<()> {
    let width = px(960.).min(screen.width - px(32.)).max(px(1.));
    let height = px(680.).min(screen.height - px(88.)).max(px(1.));
    let origin = point((screen.width - width) / 2., (screen.height - height) / 2.);
    cx.open_window(
        WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Rshot · 截图编辑".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin,
                size: size(width, height),
            })),
            display_id: Some(display),
            window_min_size: Some(size(px(620.).min(width), px(320.).min(height))),
            ..Default::default()
        },
        |window, cx| {
            cx.new(|cx| {
                let focus = cx.focus_handle();
                focus.focus(window);
                let texture = render_image(image.clone());
                Editor {
                    document: Document::new(image),
                    texture,
                    scale,
                    focus,
                    tool: Tool::Arrow,
                    drag: None,
                    busy: false,
                    status: None,
                }
            })
        },
    )?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tool {
    Arrow,
    Text,
    Mosaic,
}

impl Tool {
    fn instructions(self) -> &'static str {
        match self {
            Self::Arrow => "从箭尾拖到箭头指向的位置",
            Self::Text => "点击图片放置文字",
            Self::Mosaic => "拖出需要打码的区域",
        }
    }
}

struct Editor {
    document: Document,
    texture: Arc<RenderImage>,
    scale: f32,
    focus: FocusHandle,
    tool: Tool,
    drag: Option<(annotation::Point, annotation::Point)>,
    busy: bool,
    status: Option<SharedString>,
}

impl Editor {
    fn redraw(&mut self, cx: &mut Context<Self>) {
        self.texture = render_image((**self.document.result()).clone());
        self.status = None;
        cx.notify();
    }

    fn set_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.drag = None;
        self.tool = tool;
        self.status = None;
        self.focus.focus(window);
        cx.notify();
    }

    fn history(&mut self, redo: bool, cx: &mut Context<Self>) {
        if self.busy || self.drag.is_some() {
            return;
        }
        let changed = if redo {
            self.document.redo()
        } else {
            self.document.undo()
        };
        if changed {
            self.redraw(cx);
        }
    }

    fn start(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let image = self.document.result().dimensions();
        let bounds = image_bounds(window.viewport_size(), image);
        if self.busy || !bounds.contains(&at) {
            return;
        }
        let position = image_point(at, bounds, image);
        self.status = None;
        self.focus.focus(window);
        if self.tool == Tool::Text {
            self.add_text(position, window, cx);
        } else {
            self.drag = Some((position, position));
            cx.notify();
        }
    }

    fn update_drag(&mut self, at: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        if let Some((_, end)) = &mut self.drag {
            let image = self.document.result().dimensions();
            *end = image_point(at, image_bounds(window.viewport_size(), image), image);
            cx.notify();
        }
    }

    fn finish_drag(&mut self, at: Point<Pixels>, window: &Window, cx: &mut Context<Self>) {
        let Some((start, _)) = self.drag.take() else {
            return;
        };
        let image = self.document.result().dimensions();
        let end = image_point(at, image_bounds(window.viewport_size(), image), image);
        let mark = match self.tool {
            Tool::Arrow if start != end => Some(Mark::Arrow {
                start,
                end,
                stroke: f64::from(self.scale) * 3.5,
            }),
            Tool::Mosaic if start.0 != end.0 && start.1 != end.1 => Some(Mark::Mosaic {
                start,
                end,
                block: (self.scale * 12.).round() as u32,
            }),
            _ => None,
        };
        if let Some(mark) = mark {
            self.document.add(mark);
            self.redraw(cx);
        } else {
            cx.notify();
        }
    }

    fn add_text(
        &mut self,
        position: annotation::Point,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let parent = match text_sheet_parent(window) {
            Ok(parent) => parent,
            Err(error) => {
                self.report(error, cx);
                return;
            }
        };
        self.busy = true;
        cx.notify();
        cx.spawn_in(window, async move |editor, cx| {
            let text = text_sheet(&parent).await;
            let _ = editor.update_in(cx, |editor, window, cx| {
                editor.busy = false;
                editor.focus.focus(window);
                match text {
                    Ok(Some(text)) => match annotation::text_label(
                        &text,
                        editor.scale,
                        editor.document.result().dimensions(),
                    ) {
                        Ok(label) => {
                            editor.document.add(Mark::Text { position, label });
                            editor.redraw(cx);
                        }
                        Err(error) => editor.report(error, cx),
                    },
                    Ok(None) => cx.notify(),
                    Err(error) => editor.report(error, cx),
                }
            });
        })
        .detach();
    }

    fn report(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        self.status = Some(format!("{error:#}").into());
        cx.notify();
    }

    fn copy(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.busy || self.drag.is_some() {
            return;
        }
        self.busy = true;
        self.status = Some("正在生成 PNG…".into());
        cx.notify();
        let image = Arc::clone(self.document.result());
        let task = cx.background_spawn(async move { png_bytes(&image) });
        cx.spawn_in(window, async move |editor, cx| {
            let result = task.await;
            let _ = editor.update_in(cx, |editor, _, cx| {
                editor.busy = false;
                match result {
                    Ok(bytes) => {
                        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                            ImageFormat::Png,
                            bytes,
                        )));
                        editor.status = Some("已复制标注后的截图".into());
                        cx.notify();
                    }
                    Err(error) => editor.report(error, cx),
                }
            });
        })
        .detach();
    }

    fn save(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.busy || self.drag.is_some() {
            return;
        }
        self.busy = true;
        self.status = Some("正在保存 PNG…".into());
        cx.notify();
        let image = Arc::clone(self.document.result());
        let directory = std::env::home_dir().unwrap_or_else(|| ".".into());
        let dialog = cx.prompt_for_new_path(&directory, Some("Rshot-annotated.png"));
        cx.spawn_in(window, async move |editor, cx| {
            let result = match dialog.await {
                Ok(Ok(Some(path))) => {
                    cx.background_spawn(async move {
                        image
                            .save_with_format(&path, image::ImageFormat::Png)
                            .with_context(|| format!("无法保存 PNG 到 {}", path.display()))
                            .map(|()| true)
                    })
                    .await
                }
                Ok(Ok(None)) => Ok(false),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(anyhow!(error)),
            };
            let _ = editor.update_in(cx, |editor, window, cx| {
                editor.busy = false;
                editor.focus.focus(window);
                match result {
                    Ok(saved) => {
                        editor.status = saved.then(|| "已保存标注后的截图".into());
                        cx.notify();
                    }
                    Err(error) => editor.report(error, cx),
                }
            });
        })
        .detach();
    }
}

fn image_bounds(view: Size<Pixels>, (width, height): (u32, u32)) -> Bounds<Pixels> {
    let stage_width = (view.width - px(2. * MARGIN)).max(px(1.));
    let stage_height = (view.height - px(TOOLBAR + FOOTER + 2. * MARGIN)).max(px(1.));
    let scale = (f32::from(stage_width) / width as f32)
        .min(f32::from(stage_height) / height as f32)
        .min(1.);
    let size = size(px(width as f32 * scale), px(height as f32 * scale));
    let origin = point(
        (view.width - size.width) / 2.,
        px(TOOLBAR + MARGIN) + (stage_height - size.height) / 2.,
    );
    Bounds { origin, size }
}

fn image_point(
    at: Point<Pixels>,
    bounds: Bounds<Pixels>,
    (width, height): (u32, u32),
) -> annotation::Point {
    let x = f32::from(at.x - bounds.origin.x) * width as f32 / f32::from(bounds.size.width);
    let y = f32::from(at.y - bounds.origin.y) * height as f32 / f32::from(bounds.size.height);
    (
        (x.floor() as u32).min(width - 1),
        (y.floor() as u32).min(height - 1),
    )
}

fn text_sheet_parent(window: &Window) -> Result<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|error| anyhow!("无法读取编辑窗口 ({error:?})"))?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err(anyhow!("无法读取编辑窗口"));
    };
    // AppKit's view belongs to the active GPUI window. Retain its NSWindow for the sheet.
    unsafe {
        let view = handle.ns_view.cast::<objc2_app_kit::NSView>().as_ref();
        let native: *mut NSWindow = msg_send![view, window];
        Retained::retain(native).context("编辑窗口已关闭")
    }
}

async fn text_sheet(parent: &NSWindow) -> Result<Option<String>> {
    let main_thread = MainThreadMarker::new().context("文字输入需要主线程")?;
    let alert = NSAlert::new(main_thread);
    alert.setMessageText(&NSString::from_str("添加文字"));
    alert.addButtonWithTitle(&NSString::from_str("添加"));
    alert
        .addButtonWithTitle(&NSString::from_str("取消"))
        .setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let input = NSTextField::initWithFrame(
        NSTextField::alloc(main_thread),
        NSRect::new(NSPoint::new(0., 0.), NSSize::new(320., 26.)),
    );
    input.setPlaceholderString(Some(&NSString::from_str("输入标注文字")));
    alert.setAccessoryView(Some(&input));
    alert.layout();
    alert.window().setInitialFirstResponder(Some(&input));
    let (sender, receiver) = oneshot::channel();
    let sender = Cell::new(Some(sender));
    let completion = RcBlock::new(move |response| {
        if let Some(sender) = sender.take() {
            let _ = sender.send(response);
        }
    });
    alert.beginSheetModalForWindow_completionHandler(parent, Some(&completion));
    if receiver.await? != NSAlertFirstButtonReturn {
        return Ok(None);
    }
    let value = input.stringValue().to_string();
    let value = value.trim();
    anyhow::ensure!(value.chars().nth(500).is_none(), "文字最多支持 500 个字符");
    Ok((!value.is_empty()).then(|| value.to_string()))
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let image = self.document.result().dimensions();
        let bounds = image_bounds(window.viewport_size(), image);
        let mut root = div()
            .size_full()
            .relative()
            .overflow_hidden()
            .track_focus(&self.focus)
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(
                place(bounds).cursor_crosshair().child(
                    img(self.texture.clone())
                        .size_full()
                        .object_fit(ObjectFit::Fill),
                ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.start(event.position, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.update_drag(event.position, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_drag(event.position, window, cx)
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, window, cx| {
                    this.finish_drag(event.position, window, cx)
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = &event.keystroke;
                match (key.key.as_str(), key.modifiers.platform) {
                    ("escape", _) if !this.busy => {
                        if this.drag.take().is_some() {
                            cx.notify();
                        } else {
                            close_window(window);
                        }
                    }
                    ("z", true) => this.history(key.modifiers.shift, cx),
                    ("c", true) => this.copy(window, cx),
                    ("s", true) => this.save(window, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }));

        if let Some((start, end)) = self.drag {
            let to_canvas = |(x, y): annotation::Point| {
                point(
                    bounds.origin.x + bounds.size.width * ((x as f32 + 0.5) / image.0 as f32),
                    bounds.origin.y + bounds.size.height * ((y as f32 + 0.5) / image.1 as f32),
                )
            };
            let (a, b) = (to_canvas(start), to_canvas(end));
            match self.tool {
                Tool::Arrow => {
                    let stroke =
                        (3.5 * self.scale * f32::from(bounds.size.width) / image.0 as f32).max(1.);
                    root = root.child(
                        canvas(
                            |_, _, _| (),
                            move |_, (), window, _| {
                                let pair = |p: Point<Pixels>| {
                                    (f64::from(f32::from(p.x)), f64::from(f32::from(p.y)))
                                };
                                let mut path = PathBuilder::stroke(px(stroke));
                                for (a, b) in
                                    annotation::arrow_segments(pair(a), pair(b), f64::from(stroke))
                                {
                                    path.move_to(point(px(a.0 as f32), px(a.1 as f32)));
                                    path.line_to(point(px(b.0 as f32), px(b.1 as f32)));
                                }
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, rgb(GUIDE_COLOR));
                                }
                            },
                        )
                        .absolute()
                        .size_full(),
                    );
                }
                Tool::Mosaic => {
                    let rect = selection_rect(a, b, window.viewport_size());
                    root = root.child(
                        place(rect)
                            .border_1()
                            .border_color(rgb(GUIDE_COLOR))
                            .bg(rgba(0xff556644)),
                    );
                }
                Tool::Text => {}
            }
        }

        let mut toolbar = div()
            .absolute()
            .top_0()
            .w_full()
            .h(px(TOOLBAR))
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .bg(rgb(PANEL))
            .text_sm();
        for (tool, id, label) in [
            (Tool::Arrow, "arrow", "箭头"),
            (Tool::Text, "text", "文字"),
            (Tool::Mosaic, "mosaic", "马赛克"),
        ] {
            toolbar =
                toolbar.child(button(id, label, self.tool == tool).on_click(
                    cx.listener(move |this, _, window, cx| this.set_tool(tool, window, cx)),
                ));
        }
        root.child(
            toolbar
                .child(div().flex_1())
                .child(
                    button("undo", "撤销", false)
                        .opacity(if self.document.can_undo() { 1. } else { 0.4 })
                        .on_click(cx.listener(|this, _, _, cx| this.history(false, cx))),
                )
                .child(
                    button("redo", "重做", false)
                        .opacity(if self.document.can_redo() { 1. } else { 0.4 })
                        .on_click(cx.listener(|this, _, _, cx| this.history(true, cx))),
                )
                .child(
                    button("copy-edited", "复制", false)
                        .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))),
                )
                .child(
                    button("save-edited", "保存 PNG", true)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
        )
        .child(
            div()
                .absolute()
                .bottom_0()
                .w_full()
                .h(px(FOOTER))
                .flex()
                .items_center()
                .justify_between()
                .gap_2()
                .px_4()
                .text_sm()
                .text_color(rgb(MUTED))
                .child(super::ui_label(
                    self.status
                        .clone()
                        .unwrap_or_else(|| self.tool.instructions().into()),
                    MUTED,
                    13,
                    (f32::from(window.viewport_size().width) - 200.).max(80.) as u32,
                ))
                .child(super::ui_label(
                    format!("{} × {} px", image.0, image.1),
                    MUTED,
                    13,
                    150,
                )),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size};

    use super::{image_bounds, image_point};

    #[test]
    fn preview_coordinates_round_trip_at_edges_and_center() {
        let image = (1920, 1080);
        let bounds = image_bounds(size(px(960.), px(680.)), image);
        assert_eq!(image_point(bounds.origin, bounds, image), (0, 0));
        assert_eq!(
            image_point(
                bounds.origin + point(bounds.size.width / 2., bounds.size.height / 2.),
                bounds,
                image
            ),
            (960, 540)
        );
        assert_eq!(
            image_point(point(px(1500.), px(1100.)), bounds, image),
            (1919, 1079)
        );
    }
}
