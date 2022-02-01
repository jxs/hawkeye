use crate::slate::load_img;
use color_eyre::Result;
use dssim::{DssimImage, ToRGBAPLU, RGBAPLU};
use hawkeye_core::models::{Transition, VideoMode};
use imgref::{Img, ImgVec};
use itertools::Itertools;
use load_image::{Image, ImageData};
use std::borrow::BorrowMut;

const BLACK_SLATE: &str = "black_slate";

pub struct Slate {
    slate: DssimImage<f32>,
    similarity_algorithm: dssim::Dssim,
    // TODO: This should probably be a ref. Needs a lifetime specifier.
    pub(crate) transition: Option<Transition>,
}

impl Slate {
    /// Create a new Slate using the image bytes and the selected similarity algorithm.
    /// Note: similarity_algorithm can only be `dssim::Dssim` at the moment, so this is
    /// essentially hardcoded in type and value.
    pub fn new(slate_url: &str, transition: Option<Transition>) -> Result<Self> {
        // There's only one algo at the moment, so hardcode it instead of making it an
        // argument.
        let similarity_algorithm = dssim::Dssim::new();
        let slate_vec = load_img(slate_url)?;
        let mut slate_img = load_data(slate_vec.as_slice())?;
        // Attempt to create a sub image if a bbox was supplied for a SlateContext.
        let slate = transition
            .as_ref()
            .and_then(|transition| match &transition.to {
                VideoMode::Slate { bbox, .. } => bbox.as_ref().map(|bb| {
                    let sub_image = slate_img.borrow_mut().sub_image(
                        bb.origin[0] as usize,
                        bb.origin[1] as usize,
                        bb.bbox_width as usize,
                        bb.bbox_height as usize,
                    );
                    log::warn!("creating image from sub image...");
                    similarity_algorithm.create_image(&sub_image).unwrap()
                }),
                _ => None,
            })
            .or_else(|| similarity_algorithm.create_image(&slate_img))
            .unwrap();

        Ok(Self {
            slate,
            transition,
            similarity_algorithm,
        })
    }

    /// Compare the the slate's image to the frame's image via DSSIM.
    pub fn is_match(&self, frame: &DssimImage<f32>) -> (bool, u32) {
        let (res, _) = self.similarity_algorithm.compare(&self.slate, frame);
        let val: f64 = res.into();
        let val = (val * 1000f64) as u32;

        (val <= 900u32, val)
    }
}

/// Provide functionality to match a set of slates to an incoming image_buffer, typically from a
/// frame in a stream of video.
pub struct SlateDetector {
    slates: Vec<Slate>,
    // TODO: this should either be here (ideal) or on each Slate.
    similarity_algorithm: dssim::Dssim,
}

impl SlateDetector {
    pub fn new(slates: Vec<Slate>) -> Result<Self> {
        // There's only one algo at the moment, so hardcode it instead of making it an argument.
        let similarity_algorithm = dssim::Dssim::new();

        Ok(Self {
            slates,
            similarity_algorithm,
        })
    }

    /// Attempt to find a slate that most closely resembles the incoming
    /// `image_buffer`. If there's more than a single match, the one with lowest score
    /// is taken (the "most" matched).
    pub fn matched_slate(&self, image_buffer: &[u8]) -> Option<&Slate> {
        let mut frame_img = load_data(image_buffer).unwrap();
        // let frame = self.similarity_algorithm.create_image(&frame_img).unwrap();
        self.slates
            .iter()
            .filter_map(|slate| {
                let frame = slate
                    .transition
                    .as_ref()
                    .and_then(|transition| match &transition.to {
                        VideoMode::Slate { bbox, .. } => bbox.as_ref().map(|bb| {
                            let sub_image = frame_img.borrow_mut().sub_image(
                                bb.origin[0] as usize,
                                bb.origin[1] as usize,
                                bb.bbox_width as usize,
                                bb.bbox_height as usize,
                            );
                            log::warn!("creating image from sub image...");
                            self.similarity_algorithm.create_image(&sub_image).unwrap()
                        }),
                        _ => None,
                    })
                    .or_else(|| self.similarity_algorithm.create_image(&frame_img))
                    .unwrap();

                let (is_match, match_score) = slate.is_match(&frame);
                match is_match {
                    true => {
                        let slate_url = slate.transition.as_ref().map_or_else(
                            || BLACK_SLATE,
                            |transition| match &transition.to {
                                VideoMode::Slate { url, .. } => url,
                                _ => panic!("unknown slate?"),
                            },
                        );

                        log::debug!(
                            "is_match matched a slate: score={} url={:?}",
                            match_score,
                            slate_url,
                        );
                        Some((slate, match_score, slate_url))
                    }
                    false => None,
                }
            })
            .sorted_by_key(|slate_score_data| slate_score_data.1)
            .next()
            .map(|(slate, score, slate_url)| {
                log::debug!(
                    "is_match winning matched slate: score={} url={}",
                    score,
                    slate_url,
                );

                slate
            })
    }
}

fn load_data(data: &[u8]) -> Result<ImgVec<RGBAPLU>> {
    let img = load_image::load_data(data)?;
    Ok(match_img_bitmap(img))
}

fn match_img_bitmap(img: Image) -> ImgVec<RGBAPLU> {
    match img.bitmap {
        ImageData::RGB8(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::RGB16(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::RGBA8(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::RGBA16(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::GRAY8(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::GRAY16(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::GRAYA8(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
        ImageData::GRAYA16(ref bitmap) => Img::new(bitmap.to_rgbaplu(), img.width, img.height),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::io::Read;
    use std::path::Path;

    fn read_bytes<P: AsRef<Path>>(path: P) -> Vec<u8> {
        let mut slate_img =
            std::fs::File::open(path).expect("We must have this image in the /resources folder");
        let mut buffer = Vec::new();
        slate_img.read_to_end(&mut buffer).unwrap();
        buffer
    }

    #[test]
    fn compare_equal_images() {
        let slate = Slate::new(
            "../resources/slate_fixtures/slate-0-cbsaa-213x120.jpg",
            None,
        )
        .unwrap();
        let detector = SlateDetector::new(vec![slate]).unwrap();
        let slate_img = read_bytes("../resources/slate_fixtures/slate-0-cbsaa-213x120.jpg");
        let matched_slate = detector.matched_slate(slate_img.as_slice());

        assert!(matched_slate.is_some())
    }

    #[test]
    fn compare_diff_images() {
        let slate = Slate::new(
            "../resources/slate_fixtures/slate-0-cbsaa-213x120.jpg",
            None,
        )
        .unwrap();
        let detector = SlateDetector::new(vec![slate]).unwrap();
        let frame_img = read_bytes("../resources/slate_fixtures/non-slate-213x120.jpg");
        let matched_slate = detector.matched_slate(frame_img.as_slice());

        assert!(matched_slate.is_none())
    }
}
