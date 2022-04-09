mod actions;
mod config;
mod img_detector;
mod metrics;
mod slate;
mod video_stream;

use crate::actions::{ActionExecutor, Executors};
use crate::config::AppConfig;
use crate::img_detector::SlateDetector;
use crate::metrics::run_metrics_service;
use crate::video_stream::{process_frames, VideoStream};
use color_eyre::Result;
use crossbeam::channel::unbounded;
use gstreamer as gst;
use hawkeye_core::models::Watcher;
use hawkeye_core::utils::maybe_bootstrap_sentry;
use log::info;
use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use structopt::StructOpt;

fn main() -> Result<()> {
    color_eyre::install()?;

    // `sentry_client` must be in scope in main() to stay alive and functional.
    let sentry_client = maybe_bootstrap_sentry();
    if sentry_client.is_none() {
        pretty_env_logger::init();
    }

    let config: AppConfig = AppConfig::from_args();
    let watcher_config = File::open(config.watcher_path)?;
    let watcher: Watcher = serde_json::from_reader(watcher_config)?;
    watcher
        .is_valid()
        .expect("Invalid configuration for Watcher");

    info!("Initializing GStreamer..");
    gst::init().expect("Could not initialize GStreamer!");

    let (sender, receiver) = unbounded();

    info!("Loading executors..");
    let mut executors: Vec<ActionExecutor> = Vec::new();
    for transition in watcher.transitions.iter() {
        let mut execs: Executors = transition.clone().into();
        executors.append(&mut execs.0);
    }

    thread::spawn(move || {
        let mut runtime = actions::Runtime::new(receiver, executors);

        info!("Starting actions runtime..");
        runtime
            .run_blocking()
            .expect("Actions runtime ended unexpectedly!");
    });

    // starts metrics web app
    let metrics_port = watcher.source.ingest_port as u16;
    thread::spawn(move || run_metrics_service(metrics_port));

    let running = Arc::new(AtomicBool::new(true));

    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting termination handler");

    let detector = SlateDetector::new(&slate::load_img(watcher.slate_url.as_str())?)?;

    let server = VideoStream::new(
        watcher.source.ingest_port,
        watcher.source.container,
        watcher.source.codec,
    ).expect("Could not start video stream");
    log::info!(
        "Starting pipeline at rtp://0.0.0.0:{}",
        watcher.source.ingest_port
    );

    process_frames(server.into_iter(), detector, running, sender)
}
