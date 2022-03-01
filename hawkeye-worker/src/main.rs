mod actions;
mod config;
mod img_detector;
mod metrics;
mod slate;
mod video_stream;

use crate::actions::{ActionExecutor, Executors};
use crate::config::AppConfig;
use crate::img_detector::{Slate, SlateDetector};
use crate::metrics::run_metrics_service;
use crate::video_stream::{process_frames, RtpServer};
use color_eyre::Result;
use crossbeam::channel::unbounded;
use gstreamer as gst;
use hawkeye_core::models::{VideoMode, Watcher};
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
        info!("Initializing Sentry...");
        pretty_env_logger::init();
    }

    // Initialize an instance of a Watcher.
    info!("Initializing a Watcher instance...");
    let config: AppConfig = AppConfig::from_args();
    let watcher_config = File::open(config.watcher_path)?;
    let watcher: Watcher = serde_json::from_reader(watcher_config)?;
    watcher
        .is_valid()
        .expect("Invalid configuration for Watcher");

    info!("Initializing GStreamer..");
    gst::init().expect("Could not initialize GStreamer!");

    // Build a pipe for communicating to an "Actions Runtime" where the receiver is the Actions
    // Runtime and the sender is logic within `process_frames`.
    let (sender, receiver) = unbounded();

    // Convert each Transition into an Executor (a Vec<ActionExecutor).
    info!("Loading executors..");
    let mut executors: Vec<ActionExecutor> = Vec::new();

    info!("Mapping 'to' slates to transitions...");
    let to_slates: Vec<Slate> = watcher
        .transitions
        .iter()
        .filter_map(|transition| match &transition.to {
            VideoMode::Slate { url } => {
                let slate = Slate::new(
                    &slate::load_img(url.as_str()).unwrap(),
                    Some(transition.to_owned()),
                )
                .unwrap();

                let mut execs: Executors = transition.clone().into();
                // QUESTION: Do we only support 1 Action?
                executors.append(&mut execs.0);

                Some(slate)
            }
            _ => None,
        })
        .collect();

    // Start up an Actions Runtime with the built Vec<ActionExecutors> that are per-Transition.
    thread::spawn(move || {
        let mut runtime = actions::Runtime::new(receiver, executors);

        info!("Starting actions runtime..");
        runtime
            .run_blocking()
            .expect("Actions runtime ended unexpectedly!");
    });

    // Start metrics web app.
    let metrics_port = watcher.source.ingest_port as u16;
    thread::spawn(move || run_metrics_service(metrics_port));

    let running = Arc::new(AtomicBool::new(true));

    // Upon CTRL+C (aka SIGINT), signify this Watcher worker is no longer running.
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting termination handler");

    let slate_detector = SlateDetector::new(to_slates)?;

    // Configure the video stream for the Watcher to "watch" for slates of interest.
    log::info!(
        "Starting pipeline at rtp://0.0.0.0:{}",
        watcher.source.ingest_port
    );

    let server = RtpServer::new(
        watcher.source.ingest_port,
        watcher.source.container,
        watcher.source.codec,
    );

    // Process frames from a streaming video server, using a SlateDetector to eventually dispatch
    // Actions to an action sink (sender).
    process_frames(server.into_iter(), &slate_detector, running, sender)
}
