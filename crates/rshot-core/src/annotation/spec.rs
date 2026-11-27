use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;

use crate::geometry::{Point, Rect};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum AnnotateError {
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    #[error("schema_version: expected {SCHEMA_VERSION}, got {0}")]
    SchemaVersion(u32),
    #[error("marks[{index}].{field}: {message}")]
    Field {
        index: usize,
        field: String,
        message: String,
    },
    #[cfg(target_os = "macos")]
    #[error("marks[{index}].{field}: could not rasterize text")]
    Text {
        index: usize,
        field: &'static str,
        #[source]
        source: super::TextError,
    },
}

impl AnnotateError {
    pub(super) fn field(
        index: usize,
        field: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::Field {
            index,
            field: field.into(),
            message: message.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "WireSpec")]
pub struct Spec {
    pub schema_version: u32,
    pub marks: Vec<MarkSpec>,
}

impl Spec {
    pub fn from_json(json: &str) -> Result<Self, AnnotateError> {
        serde_json::from_str::<WireSpec>(json)?.try_into()
    }

    pub fn validate(&self) -> Result<(), AnnotateError> {
        validate_version(self.schema_version)?;
        let mut highest = 0;
        for (index, mark) in self.marks.iter().enumerate() {
            let rect = match mark {
                MarkSpec::Rect(mark) => Some(mark.rect),
                MarkSpec::Ellipse(mark) => Some(mark.rect),
                MarkSpec::Highlight(mark) => Some(mark.rect),
                MarkSpec::Redact(mark) => Some(mark.rect),
                _ => None,
            };
            if let Some(rect) = rect {
                for (field, dimension) in
                    [("rect.w", rect.size.width), ("rect.h", rect.size.height)]
                {
                    if dimension < 0 {
                        return Err(AnnotateError::field(index, field, "must be nonnegative"));
                    }
                }
            }
            let field = match mark {
                MarkSpec::Rect(mark) if mark.width == 0 => "width",
                MarkSpec::Ellipse(mark) if mark.width == 0 => "width",
                MarkSpec::Arrow(mark) if mark.width == 0 => "width",
                MarkSpec::Text(mark) if mark.size == 0 => "size",
                MarkSpec::Step(mark) if mark.size == 0 => "size",
                MarkSpec::Step(mark) => {
                    step_number(index, mark.n, &mut highest)?;
                    continue;
                }
                MarkSpec::Highlight(mark) if !(0.0..=1.0).contains(&mark.opacity) => {
                    return Err(AnnotateError::field(
                        index,
                        "opacity",
                        "must be between 0 and 1",
                    ));
                }
                _ => continue,
            };
            return Err(AnnotateError::field(
                index,
                field,
                "must be greater than zero",
            ));
        }
        Ok(())
    }
}

pub(super) fn step_number(
    index: usize,
    n: Option<u32>,
    highest: &mut u32,
) -> Result<u32, AnnotateError> {
    let n = match n {
        Some(0) => {
            return Err(AnnotateError::field(
                index,
                "n",
                "must be greater than zero",
            ));
        }
        Some(n) => n,
        None => highest.checked_add(1).ok_or_else(|| {
            AnnotateError::field(index, "n", "the next automatic number exceeds u32::MAX")
        })?,
    };
    *highest = (*highest).max(n);
    Ok(n)
}

#[derive(Deserialize)]
struct WireSpec {
    schema_version: u32,
    marks: Vec<Value>,
}

impl TryFrom<WireSpec> for Spec {
    type Error = AnnotateError;

    fn try_from(wire: WireSpec) -> Result<Self, Self::Error> {
        validate_version(wire.schema_version)?;
        let marks = wire
            .marks
            .into_iter()
            .enumerate()
            .map(|(index, value)| MarkSpec::parse(index, value))
            .collect::<Result<_, _>>()?;
        let spec = Self {
            schema_version: wire.schema_version,
            marks,
        };
        spec.validate()?;
        Ok(spec)
    }
}

fn validate_version(version: u32) -> Result<(), AnnotateError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(AnnotateError::SchemaVersion(version))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkKind {
    Rect,
    Ellipse,
    Arrow,
    Text,
    Step,
    Highlight,
    Redact,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MarkSpec {
    Rect(RectSpec),
    Ellipse(EllipseSpec),
    Arrow(ArrowSpec),
    Text(TextSpec),
    Step(StepSpec),
    Highlight(HighlightSpec),
    Redact(RedactSpec),
}

impl MarkSpec {
    pub fn kind(&self) -> MarkKind {
        match self {
            Self::Rect(_) => MarkKind::Rect,
            Self::Ellipse(_) => MarkKind::Ellipse,
            Self::Arrow(_) => MarkKind::Arrow,
            Self::Text(_) => MarkKind::Text,
            Self::Step(_) => MarkKind::Step,
            Self::Highlight(_) => MarkKind::Highlight,
            Self::Redact(_) => MarkKind::Redact,
        }
    }

    fn parse(index: usize, value: Value) -> Result<Self, AnnotateError> {
        let tag = value
            .get("type")
            .ok_or_else(|| AnnotateError::field(index, "type", "missing field"))?;
        let kind = MarkKind::deserialize(tag)
            .map_err(|error| AnnotateError::field(index, "type", error.to_string()))?;
        fn fields<T: DeserializeOwned>(index: usize, value: Value) -> Result<T, AnnotateError> {
            serde_path_to_error::deserialize(value).map_err(|error| {
                let mut field = error.path().to_string();
                let message = error.inner().to_string();
                if let Some(missing) = message
                    .strip_prefix("missing field `")
                    .and_then(|name| name.strip_suffix('`'))
                {
                    if field == "." {
                        field = missing.to_owned();
                    } else {
                        field.push('.');
                        field.push_str(missing);
                    }
                }
                AnnotateError::field(index, field, message)
            })
        }
        Ok(match kind {
            MarkKind::Rect => Self::Rect(fields(index, value)?),
            MarkKind::Ellipse => Self::Ellipse(fields(index, value)?),
            MarkKind::Arrow => Self::Arrow(fields(index, value)?),
            MarkKind::Text => Self::Text(fields(index, value)?),
            MarkKind::Step => Self::Step(fields(index, value)?),
            MarkKind::Highlight => Self::Highlight(fields(index, value)?),
            MarkKind::Redact => Self::Redact(fields(index, value)?),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RectSpec {
    pub rect: Rect<i32>,
    #[serde(default = "red")]
    pub stroke: Color,
    #[serde(default = "outline_width")]
    pub width: u32,
    #[serde(default)]
    pub radius: u32,
    #[serde(default)]
    pub fill: Option<Color>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EllipseSpec {
    pub rect: Rect<i32>,
    #[serde(default = "blue")]
    pub stroke: Color,
    #[serde(default = "outline_width")]
    pub width: u32,
    #[serde(default)]
    pub fill: Option<Color>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ArrowSpec {
    pub from: Point<i32>,
    pub to: Point<i32>,
    #[serde(default = "red")]
    pub stroke: Color,
    #[serde(default = "arrow_width")]
    pub width: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TextSpec {
    pub at: Point<i32>,
    #[serde(default)]
    pub text: String,
    #[serde(default = "text_size")]
    pub size: u32,
    #[serde(default = "charcoal")]
    pub color: Color,
    #[serde(default)]
    pub background: Option<Color>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StepSpec {
    /// Center of the circular badge.
    pub at: Point<i32>,
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default = "step_size")]
    pub size: u32,
    #[serde(default = "blue")]
    pub fill: Color,
    #[serde(default = "white")]
    pub color: Color,
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HighlightSpec {
    pub rect: Rect<i32>,
    #[serde(default = "yellow")]
    pub color: Color,
    #[serde(default = "highlight_opacity")]
    pub opacity: f64,
    #[serde(default)]
    pub blend: HighlightBlend,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HighlightBlend {
    #[default]
    Multiply,
    Normal,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RedactSpec {
    pub rect: Rect<i32>,
    #[serde(default = "black")]
    pub fill: Color,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(try_from = "String", into = "String")]
pub struct Color(pub [u8; 4]);

#[derive(Debug, Error)]
#[error("expected a hex color in #RGB, #RRGGBB, or #RRGGBBAA format")]
pub struct ColorError;

impl TryFrom<String> for Color {
    type Error = ColorError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let digits = value.strip_prefix('#').ok_or(ColorError)?;
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ColorError);
        }
        let mut rgba = [255; 4];
        match digits.len() {
            3 => {
                for (channel, byte) in rgba.iter_mut().zip(digits.bytes()) {
                    *channel = char::from(byte).to_digit(16).ok_or(ColorError)? as u8 * 17;
                }
            }
            6 | 8 => {
                for (index, channel) in rgba.iter_mut().take(digits.len() / 2).enumerate() {
                    *channel = u8::from_str_radix(&digits[index * 2..index * 2 + 2], 16)
                        .map_err(|_| ColorError)?;
                }
            }
            _ => return Err(ColorError),
        }
        Ok(Self(rgba))
    }
}

impl From<Color> for String {
    fn from(Color([r, g, b, a]): Color) -> Self {
        if a == 255 {
            format!("#{r:02X}{g:02X}{b:02X}")
        } else {
            format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
        }
    }
}

fn red() -> Color {
    Color([230, 57, 70, 255])
}
fn blue() -> Color {
    Color([39, 110, 241, 255])
}
fn charcoal() -> Color {
    Color([17, 17, 17, 255])
}
fn white() -> Color {
    Color([255, 255, 255, 255])
}
fn yellow() -> Color {
    Color([255, 216, 78, 255])
}
fn black() -> Color {
    Color([0, 0, 0, 255])
}
fn outline_width() -> u32 {
    3
}
fn arrow_width() -> u32 {
    4
}
fn text_size() -> u32 {
    22
}
fn step_size() -> u32 {
    28
}
fn highlight_opacity() -> f64 {
    0.4
}
