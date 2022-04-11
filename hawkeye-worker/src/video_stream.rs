use crate::img_detector::{Slate, SlateDetector};
use crate::metrics::{
    FOUND_CONTENT_COUNTER, FOUND_SLATE_COUNTER, FRAME_PROCESSING_DURATION,
    SIMILARITY_EXECUTION_COUNTER, SIMILARITY_EXECUTION_DURATION,
};
use crate::slate::SLATE_SIZE;
use color_eyre::eyre::{bail, eyre, Context, Result};
use concread::CowCell;
use crossbeam::channel::{bounded, Receiver, Sender, TryRecvError, TrySendError};
use derive_more::{Display, Error};
use gst::element_error;
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use hawkeye_core::models::{Codec, Container, VideoMode};
use lazy_static::lazy_static;
use log::{debug, info};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

lazy_static! {
    pub(crate) static ref LATEST_FRAME: CowCell<Option<Vec<u8>>> = CowCell::new(None);
}

#[derive(Debug, Display, Error)]
#[display(fmt = "Received error from {}: {} (debug: {:?})", src, error, debug)]
struct ErrorMessage {
    src: String,
    error: String,
    debug: Option<String>,
    source: glib::Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Event {
    Terminate,
    Mode(VideoMode),
}

pub fn process_frames(
    frame_source: impl Iterator<Item = Result<Option<Vec<u8>>>>,
    slate_detector: &SlateDetector,
    running: Arc<AtomicBool>,
    action_sink: Sender<TransitionChange>,
) -> Result<()> {
    log::debug!("process_frames called...");

    let black_image = include_bytes!("../../resources/slate_fixtures/black-213x120.jpg");
    let black_slate = Slate::new(black_image, None)?;
    let black_detector = SlateDetector::new(vec![black_slate])?;

    // TODO: This is just for debugging and to know if there were results from the
    // frame_source iterator without any frames.
    let mut empty_iterations = 0;

    for frame in frame_source {
        let frame_processing_timer = FRAME_PROCESSING_DURATION.start_timer();
        let local_buffer = match frame? {
            Some(contents) => {
                log::trace!("Empty iterations: {}", empty_iterations);
                empty_iterations = 0;
                contents
            }
            None => {
                if !running.load(Ordering::SeqCst) {
                    break;
                } else {
                    empty_iterations += 1;
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
        };

        let is_black = black_detector
            .matched_slate(local_buffer.as_slice())
            .is_some();

        let mut matched_slate: Option<&Slate> = None;

        if !is_black {
            let t = SIMILARITY_EXECUTION_DURATION.start_timer();
            matched_slate = slate_detector.matched_slate(local_buffer.as_slice());
            let took_in_seconds = t.stop_and_record();
            log::trace!("Similarity algorithm ran in {} seconds", took_in_seconds);
        }

        {
            // Save latest image bytes
            let mut write_txn = LATEST_FRAME.write();
            // Moves the local buffer
            *write_txn = Some(local_buffer);
            write_txn.commit();
        }

        // If the slate is black, then it clearly won't match a slate image.
        if is_black {
            continue;
        }

        // If the frame matched a slate, then start the Slate workflow.
        match matched_slate {
            Some(_) => {
                log::trace!("Found slate image in video stream!");
                FOUND_SLATE_COUNTER.inc();
                let video_mode = matched_slate
                    .and_then(|s| s.transition.as_ref())
                    .map(|t| &t.to)
                    .unwrap();
                let tchange = TransitionChange::new(Event::Mode(video_mode.to_owned()));
                action_sink.send(tchange)?;
            }
            None => {
                log::trace!("Content in video stream!");
                FOUND_CONTENT_COUNTER.inc();
                let tchange = TransitionChange::new(Event::Mode(VideoMode::Content));
                action_sink.send(tchange)?;
            }
        }

        SIMILARITY_EXECUTION_COUNTER.inc();

        // Trace the frame processing time.
        let took_in_seconds = frame_processing_timer.stop_and_record();
        log::trace!("Frame processing took {} seconds", took_in_seconds);

        // Stop running if the AtomicBool `running` is no longer truthy.
        if !running.load(Ordering::SeqCst) {
            break;
        }
    }

    info!("Stopping pipeline gracefully!");
    let tc = TransitionChange::new(Event::Terminate);
    action_sink.send(tc)?;

    Ok(())
}

/// A structure that encapsulates the Gstreamer pipeline video stream.
pub struct VideoStream {
    bus: gst::Bus,
    receiver: Receiver<Result<Option<Vec<u8>>>>,
    pipeline_description: String,
    pipeline: gst::Pipeline,
}

impl VideoStream {
    /// Create a new Gstreamer RTP server pipeline.
    pub fn new(ingest_port: u32, container: Container, codec: Codec) -> Result<Self> {
        let (width, height) = SLATE_SIZE;
        let pipeline_description = match (container, codec) {
            (Container::MpegTs, Codec::H264) => format!(
                "udpsrc port={} caps=\"application/x-rtp, media=(string)video, clock-rate=(int)90000, encoding-name=(string)MP2T, payload=(int)33\" ! .recv_rtp_sink_0 rtpbin ! rtpmp2tdepay ! tsdemux ! h264parse ! avdec_h264 ! videorate ! video/x-raw,framerate=10/1 ! videoconvert ! videoscale ! capsfilter caps=\"video/x-raw, width={}, height={}\"",
                ingest_port,
                width,
                height
            ),
            (Container::RawVideo, Codec::H264) => format!(
                "udpsrc port={} caps=\"application/x-rtp, media=(string)video, clock-rate=(int)90000, encoding-name=(string)H264, payload=(int)96\" ! rtph264depay ! decodebin ! videorate ! video/x-raw,framerate=10/1 ! videoconvert ! videoscale ! capsfilter caps=\"video/x-raw, width={}, height={}\"",
                ingest_port,
                width,
                height
            ),
            _ => bail!("Container ({:?}) and Codec ({:?}) not available", container, codec)
        };

        Self::new_from_description(pipeline_description)
    }

    /// Create a new Gstreamer pipeline from a given description.
    pub fn new_from_description<S: AsRef<str>>(pipeline_description: S) -> Result<Self> {
        let (sender, receiver) = bounded(1);
        let pipeline_description = pipeline_description.as_ref().into();

        debug!("Creating GStreamer Pipeline..");
        let pipeline = gst::parse_launch(
            format!(
                "{} ! pngenc snapshot=false ! appsink name=sink",
                pipeline_description
            )
            .as_str(),
        )
        .context("Pipeline description invalid, cannot create")?
        .downcast::<gst::Pipeline>()
        .map_err(|_| eyre!("Expected a gst::Pipeline"))?;

        // Get access to the appsink element.
        let appsink = pipeline
            .by_name("sink")
            .ok_or_else(|| eyre!("Sink element not found"))?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| eyre!("Sink element is expected to be an appsink!"))?;

        appsink
            .set_property("sync", &false)
            .context("Failed to disable gst pipeline sync")?;
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    // Pull the sample in question out of the appsink's buffer.
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer_ref = sample.buffer().ok_or_else(|| {
                        element_error!(
                            appsink,
                            gst::ResourceError::Failed,
                            ("Failed to get buffer from appsink")
                        );

                        if let Err(err) = sender.try_send(Err(color_eyre::eyre::eyre!(
                            "Failed to get buffer from appsink"
                        ))) {
                            log::error!("Could not send message in stream: {}", err)
                        }

                        gst::FlowError::Error
                    })?;

                    // At this point, buffer is only a reference to an existing memory region somewhere.
                    // When we want to access its content, we have to map it while requesting the required
                    // mode of access (read, read/write).
                    // This type of abstraction is necessary, because the buffer in question might not be
                    // on the machine's main memory itself, but rather in the GPU's memory.
                    // So mapping the buffer makes the underlying memory region accessible to us.
                    // See: https://gstreamer.freedesktop.org/documentation/plugin-development/advanced/allocation.html
                    let buffer = buffer_ref.map_readable().map_err(|_| {
                        element_error!(
                            appsink,
                            gst::ResourceError::Failed,
                            ("Failed to map buffer readable")
                        );

                        if let Err(err) = sender.try_send(Err(color_eyre::eyre::eyre!(
                            "Failed to map buffer readable"
                        ))) {
                            log::error!("Could not send message in stream: {}", err)
                        }

                        gst::FlowError::Error
                    })?;
                    log::trace!("Frame extracted from pipeline");

                    match sender.try_send(Ok(Some(buffer.to_vec()))) {
                        Ok(_) => Ok(gst::FlowSuccess::Ok),
                        Err(TrySendError::Full(_)) => {
                            log::trace!("Channel is full, discarded frame");
                            Ok(gst::FlowSuccess::Ok)
                        }
                        Err(TrySendError::Disconnected(_)) => {
                            log::debug!("Returning EOS in pipeline callback fn");
                            Err(gst::FlowError::Eos)
                        }
                    }
                })
                .build(),
        );

        let bus = pipeline
            .bus()
            .ok_or_else(|| eyre!("Pipeline without bus. Shouldn't happen!"))?;

        pipeline
            .set_state(gst::State::Playing)
            .context("Cannot start pipeline")?;
        info!("Pipeline started: {}", pipeline_description);

        Ok(Self {
            bus,
            pipeline,
            pipeline_description,
            receiver,
        })
    }
}

impl Iterator for VideoStream {
    type Item = Result<Option<Vec<u8>>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.receiver.try_recv() {
            Ok(event) => return Some(event),
            Err(TryRecvError::Empty) => {
                // Check if there are errors in the GStreamer pipeline itself.
                if let Some(msg) = self.bus.pop() {
                    use gst::MessageView;

                    match msg.view() {
                        MessageView::Eos(..) => {
                            // The End-of-stream message is posted when the stream is done, which in our case
                            // happens immediately after matching the slate image because we return
                            // gst::FlowError::Eos then.
                            return None;
                        }
                        MessageView::Error(err) => {
                            let error_msg = ErrorMessage {
                                src: msg
                                    .src()
                                    .map(|s| String::from(s.path_string()))
                                    .unwrap_or_else(|| String::from("None")),
                                error: err.error().to_string(),
                                debug: err.debug(),
                                source: err.error(),
                            };
                            log::error!("Error returned by pipeline: {:?}", error_msg);
                            // TODO: Should return a proper error here, returning `None` will simply stop the iterator.
                            return None;
                        }
                        _ => (),
                    }
                }
            }
            Err(TryRecvError::Disconnected) => {
                log::debug!(
                    "The Pipeline channel is disconnected: {}",
                    self.pipeline_description
                );
                return None;
            }
        }
        // Nothing to report in this iteration.
        // Frames could not be captured, but there are no errors in the pipeline.
        Some(Ok(None))
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        match self.pipeline.set_state(gst::State::Null) {
            Ok(_) => log::debug!("Pipeline stopped!"),
            Err(err) => log::error!("Could not stop pipeline: {}", err),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct TransitionChange {
    pub event: Event,
}

impl TransitionChange {
    pub fn new(event: Event) -> Self {
        Self { event }
    }
}
