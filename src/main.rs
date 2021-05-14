#![feature(box_syntax, slice_group_by)]

use nightfall::*;

use slog::o;
use slog::Drain;

use sectionizer::Sectionizer;

#[tokio::main]
async fn main() {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    let logger = slog::Logger::root(drain, o!());

    let mut args = std::env::args();

    if args.len() < 3 {
        slog::error!(logger, "Usage: sectionizer <target> <reference>");
        return;
    }

    let _ = args.next();

    let file1 = args.next().unwrap();
    let file2 = args.next().unwrap();

    let state = StateManager::new(
        &mut xtra::spawn::Tokio::Global,
        "/tmp/streaming_cache".into(),
        "/usr/bin/ffmpeg".into(),
        logger.clone(),
    );

    let mut sectionizer = Sectionizer::new(logger.clone(), state);

    let sections = sectionizer.categorize(file1, file2).await.unwrap();
    log_sections(sections.0, &logger);
    log_sections(sections.1, &logger);
}

fn log_sections(sections: sectionizer::Sections, logger: &slog::Logger) {
    slog::info!(logger, "Sections for {}", sections.target);

    for section in sections.sections {
        let start_ts = format!("{:02}:{:02}", section.0 / 60, section.0 % 60);
        let end_ts = format!("{:02}:{:02}", section.1 / 60, section.1 % 60);
        slog::info!(logger, "{} -> {}", start_ts, end_ts);
    }
}
