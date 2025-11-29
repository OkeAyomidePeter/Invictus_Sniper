use log::{Level, Metadata, Record, SetLoggerError};
use tokio::sync::mpsc;
use crate::gui::bot_runtime::BotEvent;

pub struct GuiLogger {
    sender: mpsc::UnboundedSender<BotEvent>,
    max_level: Level,
}

impl GuiLogger {
    pub fn init(sender: mpsc::UnboundedSender<BotEvent>, max_level: Level) -> Result<(), SetLoggerError> {
        let logger = Box::new(GuiLogger { sender, max_level });
        log::set_boxed_logger(logger)?;
        log::set_max_level(max_level.to_level_filter());
        Ok(())
    }
}

impl log::Log for GuiLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.max_level
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let message = format!("{}", record.args());
            let level = record.level().to_string();
            
            // Print to stdout as well
            println!("[{}] {}", level, message);

            // Send to GUI
            let _ = self.sender.send(BotEvent::LogMessage {
                level,
                message,
            });
        }
    }

    fn flush(&self) {}
}
