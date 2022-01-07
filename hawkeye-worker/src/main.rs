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
use hawkeye_core::models::Watcher;
use log::info;
use pretty_env_logger::env_logger;
use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use structopt::StructOpt;

fn main() -> Result<()> {
    color_eyre::install()?;
    env_logger::init_from_env(
        env_logger::Env::default().filter_or(env_logger::DEFAULT_FILTER_ENV, "info"),
    );

    // Initialize an instance of a Watcher.
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
    let mut to_slates: Vec<Slate> = Vec::new();
    for transition in watcher.transitions.iter() {
        if let Some(context) = &transition.to_context {
            if let Some(slate_context) = &context.slate_context {
                to_slates.push(Slate::new(
                    &slate::load_img(slate_context.slate_url.as_str())?,
                    Some(transition.clone()),
                )?);
            }
        }
        let mut execs: Executors = transition.clone().into();
        // QUESTION: Do we only support 1 Action?
        executors.append(&mut execs.0);
    }

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
    process_frames(server.into_iter(), slate_detector, running, sender)
}
