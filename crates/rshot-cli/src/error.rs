use rshot_core::{
    annotation::TextError,
    capture::CaptureError,
    ocr::{Match, OcrError, RecognitionLevel},
};
use serde::Serialize;

#[repr(u8)]
pub enum ExitStatus {
    Io = 1,
    Usage = 2,
    Render = 3,
    Capture = 4,
    AmbiguousTarget = 5,
    TargetNotFound = 6,
}

#[derive(Debug, Serialize)]
pub struct CliError {
    pub kind: &'static str,
    pub message: String,
    pub hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<Match>>,
    #[serde(skip)]
    exit_code: u8,
}

impl CliError {
    pub fn new(status: ExitStatus, kind: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            hint: None,
            matches: None,
            exit_code: status as u8,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_matches(mut self, matches: Vec<Match>) -> Self {
        self.matches = Some(matches);
        self
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }

    #[allow(dead_code)]
    pub fn not_implemented(command: &str) -> Self {
        Self::new(
            ExitStatus::Usage,
            "not_implemented",
            format!("The {command} command is not implemented yet."),
        )
    }
}

impl From<std::io::Error> for CliError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ExitStatus::Io, "io_error", error.to_string())
    }
}

impl From<image::ImageError> for CliError {
    fn from(error: image::ImageError) -> Self {
        match error {
            image::ImageError::IoError(error) => error.into(),
            _ => Self::new(ExitStatus::Io, "image_io_failed", error.to_string()),
        }
    }
}

impl From<CaptureError> for CliError {
    fn from(error: CaptureError) -> Self {
        match error {
            CaptureError::UnsupportedMacOs => Self::new(
                ExitStatus::Capture,
                "unsupported_macos",
                "Screen capture requires macOS 14 or later.",
            ),
            CaptureError::PermissionDenied => Self::new(
                ExitStatus::Capture,
                "permission_denied",
                "Screen recording permission is required.",
            )
            .with_hint("Grant screen recording permission in System Settings, then restart Rshot."),
            CaptureError::DisplayNotFound => Self::new(
                ExitStatus::TargetNotFound,
                "display_not_found",
                "The requested display was not found.",
            ),
            CaptureError::RegionOutOfBounds => Self::new(
                ExitStatus::Usage,
                "region_out_of_bounds",
                "The capture region is outside the bounds of the image or is empty.",
            ),
            _ => Self::new(ExitStatus::Capture, "capture_failed", error.to_string()),
        }
    }
}

impl From<OcrError> for CliError {
    fn from(error: OcrError) -> Self {
        let message = error.to_string();
        match error {
            OcrError::UnsupportedLanguage { level, .. } => {
                let error = Self::new(ExitStatus::Usage, "unsupported_language", message);
                match level {
                    RecognitionLevel::Accurate => error,
                    RecognitionLevel::Fast => error.with_hint("Retry without --fast."),
                }
            }
            OcrError::EmptyImage => Self::new(ExitStatus::Usage, "invalid_image", message),
            OcrError::UnsupportedPlatform => {
                Self::new(ExitStatus::Capture, "unsupported_macos", message)
            }
            _ => Self::new(ExitStatus::Capture, "ocr_failed", message),
        }
    }
}

impl From<TextError> for CliError {
    fn from(error: TextError) -> Self {
        Self::new(ExitStatus::Render, "text_render_failed", error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_use_the_documented_exit_codes() {
        for (status, code) in [
            (ExitStatus::Io, 1),
            (ExitStatus::Usage, 2),
            (ExitStatus::Render, 3),
            (ExitStatus::Capture, 4),
            (ExitStatus::AmbiguousTarget, 5),
            (ExitStatus::TargetNotFound, 6),
        ] {
            assert_eq!(
                CliError::new(status, "test", "Test error.").exit_code(),
                code
            );
        }
    }

    #[test]
    fn core_errors_map_to_their_cli_categories() {
        let error = CliError::from(CaptureError::PermissionDenied);
        assert_eq!(error.exit_code(), 4);
        assert_eq!(error.kind, "permission_denied");
        assert!(error.hint.is_some());
        assert_eq!(CliError::from(CaptureError::DisplayNotFound).exit_code(), 6);
        assert_eq!(
            CliError::from(CaptureError::RegionOutOfBounds).exit_code(),
            2
        );
        assert_eq!(CliError::from(TextError::TooTall).exit_code(), 3);
    }

    #[test]
    fn image_io_errors_use_the_io_exit_code() {
        let error = image::ImageError::IoError(std::io::ErrorKind::NotFound.into());
        let error = CliError::from(error);
        assert_eq!(error.exit_code(), 1);
        assert_eq!(error.kind, "io_error");
    }
}
