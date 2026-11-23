use super::{
    ACCENT, BACKGROUND, FOREGROUND, MUTED, button, capture, hide_native_window, png_bytes,
    render_image,
};
use crate::stitch::{Axis, Progress, Stitcher};
use anyhow::{Context as _, Result, anyhow};
use gpui::*;
use image::RgbaImage;
use std::sync::Arc;

pub(super) struct LongShot {
    stitcher: Option<Stitcher>,
    preview: Arc<RenderImage>,
    dimensions: (u32, u32),
    display: DisplayId,
    scale: f32,
    region: [u32; 4],
    source_dimensions: (u32, u32),
    status: SharedString,
}

impl LongShot {
    pub(super) fn new(
        axis: Axis,
        first: RgbaImage,
        display: DisplayId,
        scale: f32,
        region: [u32; 4],
        source_dimensions: (u32, u32),
    ) -> Result<Self> {
        let stitcher = Stitcher::new(axis, first)?;
        Ok(Self {
            preview: render_image(stitcher.preview(680)),
            dimensions: stitcher.dimensions(),
            stitcher: Some(stitcher),
            display,
            scale,
            region,
            source_dimensions,
            status: "滚动原窗口，然后点击「添加一帧」。".into(),
        })
    }

    pub(super) fn open(
        self,
        target: Bounds<Pixels>,
        screen: Size<Pixels>,
        cx: &mut App,
    ) -> Result<()> {
        let width = px(384.).min(screen.width);
        let height = px(330.).min(screen.height - px(64.));
        let right = target.origin.x + target.size.width;
        let x = if target.origin.x > screen.width - right {
            target.origin.x - width - px(12.)
        } else {
            right + px(12.)
        };
        let origin = point(x.clamp(px(0.), screen.width - width), px(42.));
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Rshot · 长截图".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin,
                    size: size(width, height),
                })),
                display_id: Some(self.display),
                kind: WindowKind::PopUp,
                is_resizable: false,
                is_minimizable: false,
                ..Default::default()
            },
            |_, cx| cx.new(|_| self),
        )?;
        Ok(())
    }

    fn next_frame(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(mut stitcher) = self.stitcher.take() else {
            return;
        };
        let (display, scale, region, dimensions) = (
            self.display,
            self.scale,
            self.region,
            self.source_dimensions,
        );
        self.status = "正在读取并匹配画面…".into();
        cx.notify();
        let task = cx.background_spawn(async move {
            let result = (|| -> Result<_> {
                let frame = capture::region(display.into(), scale, dimensions, region)?;
                let progress = stitcher.push(frame)?;
                let preview =
                    (progress == Progress::Extended).then(|| render_image(stitcher.preview(680)));
                Ok((progress, preview))
            })();
            (stitcher, result)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (stitcher, result) = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.dimensions = stitcher.dimensions();
                this.stitcher = Some(stitcher);
                this.status = match result {
                    Ok((progress, preview)) => {
                        if let Some(preview) = preview {
                            this.preview = preview;
                        }
                        match progress {
                            Progress::Extended => "已拼接。继续滚动或导出。".into(),
                            Progress::Unchanged => "画面没有变化，请先滚动。".into(),
                            Progress::Revisited => "已回到拼接范围内，长图尺寸不变。".into(),
                            Progress::Unmatched => {
                                "无法确定重叠区域。请回到上一次位置，再少量滚动。".into()
                            }
                        }
                    }
                    Err(error) => format!("{error:#}").into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn copy(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(stitcher) = self.stitcher.take() else {
            return;
        };
        self.status = "正在生成 PNG…".into();
        cx.notify();
        let task = cx.background_spawn(async move {
            let result = png_bytes(&stitcher.image());
            (stitcher, result)
        });
        cx.spawn_in(window, async move |this, cx| {
            let (stitcher, result) = task.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.stitcher = Some(stitcher);
                this.status = match result {
                    Ok(bytes) => {
                        cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                            ImageFormat::Png,
                            bytes,
                        )));
                        "已复制长截图。".into()
                    }
                    Err(error) => format!("{error:#}").into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.stitcher.is_none() {
            return;
        }
        if let Err(error) = hide_native_window(window) {
            self.status = format!("{error:#}").into();
            cx.notify();
            return;
        }
        let stitcher = self.stitcher.take().expect("checked above");
        let directory = std::env::home_dir().unwrap_or_else(|| ".".into());
        let dialog = cx.prompt_for_new_path(&directory, Some("Rshot-long.png"));
        cx.spawn_in(window, async move |this, cx| {
            let (stitcher, result) = match dialog.await {
                Ok(Ok(Some(path))) => {
                    cx.background_spawn(async move {
                        let result = stitcher
                            .image()
                            .save_with_format(&path, image::ImageFormat::Png)
                            .with_context(|| format!("无法保存 PNG 到 {}", path.display()))
                            .map(|()| true);
                        (stitcher, result)
                    })
                    .await
                }
                Ok(Ok(None)) => (stitcher, Ok(false)),
                Ok(Err(error)) => (stitcher, Err(error)),
                Err(error) => (stitcher, Err(anyhow!(error))),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.stitcher = Some(stitcher);
                this.status = match result {
                    Ok(true) => "已保存长截图。".into(),
                    Ok(false) => "已取消保存，可以继续拼接。".into(),
                    Err(error) => format!("{error:#}").into(),
                };
                window.activate_window();
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for LongShot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (width, height) = self.dimensions;
        div()
            .size_full()
            .flex()
            .flex_col()
            .p_3()
            .gap_2()
            .bg(rgb(BACKGROUND))
            .text_color(rgb(FOREGROUND))
            .child(super::ui_label(
                format!("{width} × {height} px"),
                ACCENT,
                13,
                180,
            ))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_hidden()
                    .child(
                        img(self.preview.clone())
                            .absolute()
                            .size_full()
                            .object_fit(ObjectFit::Contain),
                    ),
            )
            .child(div().text_sm().min_h(px(38.)).child(super::ui_label(
                self.status.clone(),
                MUTED,
                13,
                340,
            )))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .opacity(if self.stitcher.is_some() { 1. } else { 0.55 })
                    .child(
                        button("next-frame", "添加一帧", true).on_click(
                            cx.listener(|this, _, window, cx| this.next_frame(window, cx)),
                        ),
                    )
                    .child(
                        button("copy-long", "复制", false)
                            .on_click(cx.listener(|this, _, window, cx| this.copy(window, cx))),
                    )
                    .child(
                        button("save-long", "保存 PNG", false)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}
