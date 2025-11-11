use tracing::{Event, Level, Subscriber};
use tracing_subscriber::{Layer, layer::Context};

pub struct WorkerLayer {
    level: Level,
}

impl WorkerLayer {
    pub fn new(level: Level) -> Self {
        Self { level }
    }
}

pub struct StringVisitor<'a> {
    string: &'a mut String,
}

impl<'a> tracing::field::Visit for StringVisitor<'a> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        if field.name() == "message" {
            write!(self.string, "{:?}", value).ok();
        } else {
            write!(self.string, "{} = {:?}; ", field.name(), value).ok();
        }
    }
}

impl<S: Subscriber> Layer<S> for WorkerLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        // Filter by level
        if *event.metadata().level() > self.level {
            return;
        }

        let level = event.metadata().level();
        let target = event.metadata().target();
        let name = event.metadata().name();
        let mut fields = String::new();
        event.record(&mut StringVisitor {
            string: &mut fields,
        });

        // Output using Cloudflare’s built-in console
        match *level {
            Level::ERROR => worker::console_error!("{target}: {fields} ({name})"),
            Level::WARN => worker::console_warn!("{target}: {fields} ({name})"),
            Level::INFO => worker::console_log!("{level:>5} {target}: {fields} ({name})"),
            _ => worker::console_debug!("{level:>5} {target}: {fields} ({name})"),
        }
    }
}

static INIT: std::sync::Once = std::sync::Once::new();

pub fn init_tracing() {
    use tracing_subscriber::prelude::*;

    INIT.call_once(|| {
        let subscriber = tracing_subscriber::registry().with(WorkerLayer::new(Level::TRACE));

        tracing::subscriber::set_global_default(subscriber)
            .expect("Failed to set tracing subscriber");
    });
}
