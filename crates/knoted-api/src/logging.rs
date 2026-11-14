//! Logging initialisation for the API process.
//!
//! The process configures `logging.level` and `logging.format`; text output is
//! for a human at a terminal, JSON for the log collector. Initialisation is
//! split so tests can capture output in memory through
//! [`init_with_writer`] instead of parsing stderr.

use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Mutex, MutexGuard, Once, PoisonError};

use knoted_core::config::{LogFormat, LogLevel};
use thiserror::Error;
use tracing::Level;
use tracing_subscriber::fmt::MakeWriter;

/// Logging setup failures.
#[derive(Debug, Error)]
pub enum LoggingError {
    /// A subscriber was already installed in this process.
    #[error("logging is already initialised in this process")]
    AlreadyInitialised,
}

/// Installs the process-wide subscriber writing to stderr and the production
/// panic hook.
///
/// The panic hook is the reason this function differs from
/// [`init_with_writer`]: the Rust default hook prints the panic payload, which
/// can embed note text, tokens or other user data. Handling user content (T13+)
/// makes that a leak into the log stream, so the production hook replaces it
/// with a fixed message and the source location. Tests that capture logs keep
/// the standard hook by calling [`init_with_writer`].
pub fn init(level: LogLevel, format: LogFormat) -> Result<(), LoggingError> {
    init_with_writer(io::stderr, level, format)?;
    install_panic_hook();
    Ok(())
}

/// Replaces the process panic hook with one that never formats the payload.
///
/// The hook emits `process panicked` plus `file:line` through `tracing`, so it
/// lands in the configured log format and sink instead of being written to
/// stderr by the default hook. The panic payload is never read: it may contain
/// user content. The request id is not attached because tracing span fields are
/// write-only, so the hook cannot read it back; the panic guard's own log line
/// carries the id for the failing request.
///
/// Idempotent: the first installation wins, so a test and `init` cannot chain
/// hooks within one process.
pub(crate) fn install_panic_hook() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let location = info
                .location()
                .map(|location| format!("{}:{}", location.file(), location.line()));
            // A panicking log sink must not turn one panic into a process
            // abort, so emitting is best effort.
            let _ = catch_unwind(AssertUnwindSafe(|| {
                tracing::error!(
                    target: "knoted_api::panic",
                    location = location.as_deref().unwrap_or("unknown"),
                    "process panicked"
                );
            }));
        }));
    });
}

/// Installs the process-wide subscriber writing to `writer`.
///
/// Returns [`LoggingError::AlreadyInitialised`] instead of panicking when a
/// subscriber is already installed, so a test helper or a reload path cannot
/// abort the process. ANSI colours are disabled: JSON output and captured test
/// output must stay machine-readable.
pub fn init_with_writer<W>(
    writer: W,
    level: LogLevel,
    format: LogFormat,
) -> Result<(), LoggingError>
where
    W: for<'writer> MakeWriter<'writer> + Send + Sync + 'static,
{
    let builder = tracing_subscriber::fmt()
        .with_max_level(to_tracing_level(level))
        .with_ansi(false)
        .with_writer(writer);

    match format {
        LogFormat::Text => builder.try_init(),
        LogFormat::Json => builder.json().flatten_event(true).try_init(),
    }
    .map_err(|_| LoggingError::AlreadyInitialised)
}

fn to_tracing_level(level: LogLevel) -> Level {
    match level {
        LogLevel::Trace => Level::TRACE,
        LogLevel::Debug => Level::DEBUG,
        LogLevel::Info => Level::INFO,
        LogLevel::Warn => Level::WARN,
        LogLevel::Error => Level::ERROR,
    }
}

/// An in-memory log sink for tests.
///
/// The buffer is cloneable and every clone writes to the same bytes, so the
/// writer handed to [`init_with_writer`] can be a short-lived clone while the
/// test keeps one to read the captured output.
#[derive(Debug, Clone, Default)]
pub struct LogBuffer {
    bytes: Arc<Mutex<Vec<u8>>>,
}

impl LogBuffer {
    /// Creates an empty buffer.
    pub fn new() -> LogBuffer {
        LogBuffer::default()
    }

    /// Everything written so far, lossily decoded as UTF-8.
    pub fn contents(&self) -> String {
        String::from_utf8_lossy(&self.lock()).into_owned()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<u8>> {
        self.bytes.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.lock().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
