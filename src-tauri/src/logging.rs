use flexi_logger::{FileSpec, Logger, WriteMode};
use std::path::PathBuf;

/// Initialize logging. In debug mode, logs to stdout + file.
/// In release mode, logs errors only to file with rotation.
pub fn init(log_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let file_spec = FileSpec::default()
        .directory(log_dir)
        .basename("tg-korean-search");

    let logger = if cfg!(debug_assertions) {
        Logger::try_with_env_or_str("debug")?
            .log_to_file(file_spec)
            .duplicate_to_stdout(flexi_logger::Duplicate::All)
    } else {
        Logger::try_with_str("error")?
            .log_to_file(file_spec)
            .rotate(
                flexi_logger::Criterion::Size(10_000_000), // 10MB
                flexi_logger::Naming::Numbers,
                flexi_logger::Cleanup::KeepLogFiles(3),
            )
    };

    logger.write_mode(WriteMode::BufferAndFlush).start()?;

    Ok(())
}
