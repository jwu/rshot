use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct Point<T> {
    pub x: T,
    pub y: T,
}

impl<T> Point<T> {
    pub fn new(x: T, y: T) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size<T> {
    pub width: T,
    pub height: T,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(
    from = "RectFields<T>",
    into = "RectFields<T>",
    bound(
        serialize = "T: Clone + Serialize",
        deserialize = "T: Deserialize<'de>"
    )
)]
pub struct Rect<T> {
    pub origin: Point<T>,
    pub size: Size<T>,
}

#[derive(Deserialize, Serialize)]
struct RectFields<T> {
    x: T,
    y: T,
    w: T,
    h: T,
}

impl<T> From<RectFields<T>> for Rect<T> {
    fn from(fields: RectFields<T>) -> Self {
        Self::new(fields.x, fields.y, fields.w, fields.h)
    }
}

impl<T> From<Rect<T>> for RectFields<T> {
    fn from(rect: Rect<T>) -> Self {
        Self {
            x: rect.origin.x,
            y: rect.origin.y,
            w: rect.size.width,
            h: rect.size.height,
        }
    }
}

impl<T> Rect<T> {
    pub fn new(x: T, y: T, width: T, height: T) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size { width, height },
        }
    }
}
