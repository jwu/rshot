use std::{cell::RefCell, rc::Rc, sync::Arc};

use gpui::{Canvas, Corners, ObjectFit, RenderImage, canvas};

pub(super) struct Preview {
    current: Arc<RenderImage>,
    painted: Rc<RefCell<Option<Arc<RenderImage>>>>,
}

impl Preview {
    pub(super) fn new(image: Arc<RenderImage>) -> Self {
        Self {
            current: image,
            painted: Rc::default(),
        }
    }

    pub(super) fn replace(&mut self, image: Arc<RenderImage>) {
        self.current = image;
    }

    pub(super) fn element(&self, fit: ObjectFit) -> Canvas<()> {
        let image = self.current.clone();
        let painted = self.painted.clone();
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                let bounds = fit.get_bounds(bounds, image.size(0));
                if let Err(error) =
                    window.paint_image(bounds, Corners::default(), image.clone(), 0, false)
                {
                    eprintln!("Rshot: {error:#}");
                    return;
                }
                let mut painted = painted.borrow_mut();
                if painted.as_ref().is_some_and(|old| old.id == image.id) {
                    return;
                }
                if let Some(old) = painted.replace(image) {
                    // GPUI still presents the previous scene until this paint has completed.
                    window.on_next_frame(move |window, _| {
                        let _ = window.drop_image(old);
                    });
                }
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::RenderImage;
    use image::{Frame, RgbaImage};
    use std::sync::Arc;

    use super::Preview;

    #[test]
    fn replacing_unpainted_images_releases_them_immediately() {
        let first = Arc::new(RenderImage::new([Frame::new(RgbaImage::new(1, 1))]));
        let weak = Arc::downgrade(&first);
        let mut preview = Preview::new(first);
        let second = Arc::new(RenderImage::new([Frame::new(RgbaImage::new(1, 1))]));
        preview.replace(second.clone());
        assert!(weak.upgrade().is_none());
        assert_eq!(preview.current.id, second.id);
    }
}
