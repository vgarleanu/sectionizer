#![feature(box_syntax, slice_group_by)]

use nightfall::*;

use slog::o;
use slog::Drain;

use sectionizer::get_chapters;

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

    let sections = get_chapters(state, logger.clone(), file1, file2).await;

    for group in sections {
        let start_ts = format!("{:02}:{:02}", group.0 / 60, group.0 % 60);
        let end_ts = format!("{:02}:{:02}", group.1 / 60, group.1 % 60);
        slog::info!(logger, "{} -> {}", start_ts, end_ts);
    }
}
