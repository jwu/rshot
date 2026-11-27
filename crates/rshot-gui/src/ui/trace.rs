use std::{
    env,
    ffi::OsStr,
    sync::{
        LazyLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use super::platform::physical_footprint;
use gpui::{Context, Window};

#[derive(Clone, Copy)]
pub(super) struct CaptureTrace {
    id: u64,
    start: Instant,
}

impl CaptureTrace {
    pub(super) fn start() -> Option<Self> {
        static ENABLED: LazyLock<bool> =
            LazyLock::new(|| env::var_os("RSHOT_TRACE").as_deref() == Some(OsStr::new("1")));
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        (*ENABLED).then(|| {
            let trace = Self {
                id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
                start: Instant::now(),
            };
            trace.record("capture_triggered");
            trace
        })
    }

    pub(super) fn record(self, stage: &'static str) {
        let elapsed_ms = self.start.elapsed().as_secs_f64() * 1000.;
        let id = self.id;
        match physical_footprint() {
            Some(bytes) => eprintln!(
                "rshot_trace capture_id={id} stage={stage} elapsed_ms={elapsed_ms:.3} phys_footprint_bytes={bytes}"
            ),
            None => eprintln!(
                "rshot_trace capture_id={id} stage={stage} elapsed_ms={elapsed_ms:.3} phys_footprint_bytes=unavailable"
            ),
        }
    }

    pub(super) fn first_frame(self, stage: &'static str, window: &Window) {
        window.on_next_frame(move |_, cx| {
            // On macOS GPUI presents after its frame callbacks and queues foreground tasks.
            cx.spawn(async move |_| self.record(stage)).detach();
        });
    }

    pub(super) fn window_closed<T: 'static>(self, stage: &'static str, cx: &Context<T>) {
        cx.on_release(move |_, cx| cx.defer(move |_| self.record(stage)))
            .detach();
    }
}
