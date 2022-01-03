use color_eyre::Result;
use dssim::{DssimImage, ToRGBAPLU, RGBAPLU};
use hawkeye_core::models::Transition;
use imgref::{Img, ImgVec};
use itertools::Itertools;
use load_image::{Image, ImageData};

pub struct Slate {
    slate: DssimImage<f32>,
    similarity_algorithm: dssim::Dssim,
    // TODO: This should probably be a ref. Needs a lifetime specifier.
    pub(crate) transition: Option<Transition>,
}

impl Slate {
    /// Create a new Slate using the image bytes and the selected similarity algorithm.
    /// Note: similarity_algorithm can only be `dssim::Dssim` at the moment, so this is essentially
    /// hardcoded in type and value.
    pub fn new(slate_data: &[u8], transition: Option<Transition>) -> Result<Self> {
        let slate_img = load_data(slate_data)?;

        // There's only one algo at the moment, so hardcode it instead of making it an argument.
        let similarity_algorithm = dssim::Dssim::new();
        let slate = similarity_algorithm.create_image(&slate_img).unwrap();

        Ok(Self {
            slate,
            transition,
            similarity_algorithm,
        })
    }

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

    pub fn matched_slate(&self, image_buffer: &[u8]) -> Option<&Slate> {
        let frame_img = load_data(image_buffer).unwrap();
        let frame = self.similarity_algorithm.create_image(&frame_img).unwrap();
        let t = "";

        let s = self
            .slates
            .iter()
            .filter_map(|slate| {
                let (is_match, match_score) = slate.is_match(&frame);
                // let t1 = slate.transition.as_ref()
                //     .and_then(|t| t.from_context.as_ref())
                //     .and_then(|fc| fc.slate_context.as_ref())
                //     .map(|sc| sc.slate_url.as_str())
                //     .unwrap_or(t);
                // let t2 = slate.transition.as_ref()
                //     .and_then(|t| t.to_context.as_ref())
                //     .and_then(|tc| tc.slate_context.as_ref())
                //     .map(|sc| sc.slate_url.as_str())
                //     .unwrap_or(t);

                // log::info!("SLATE MATCHED: score={} from={} to={}", match_score.1, t1, t2);
                match is_match {
                    true => Some((slate, match_score)),
                    false => None,
                }
            })
            .sorted_by_key(|slate_and_score| slate_and_score.1)
            .map(|(slate, score)| {
                let from_slate_url = slate
                    .transition
                    .as_ref()
                    .and_then(|t| t.from_context.as_ref())
                    .and_then(|fc| fc.slate_context.as_ref())
                    .map(|sc| sc.slate_url.as_str())
                    .unwrap_or(t);
                let to_slate_url = slate
                    .transition
                    .as_ref()
                    .and_then(|t| t.to_context.as_ref())
                    .and_then(|tc| tc.slate_context.as_ref())
                    .map(|sc| sc.slate_url.as_str())
                    .unwrap_or(t);
                if from_slate_url.to_string().len() > 0 || to_slate_url.to_string().len() > 0 {
                    log::info!(
                        "MATCHED Slate Scores: {} from={} to={}",
                        score,
                        from_slate_url,
                        to_slate_url
                    );
                }

                (slate, score)
            })
            .next()
            .and_then(|s| Option::from(s.0));

        match s {
            Some(slate) => {
                let from_slate_url = slate
                    .transition
                    .as_ref()
                    .and_then(|t| t.from_context.as_ref())
                    .and_then(|fc| fc.slate_context.as_ref())
                    .map(|sc| sc.slate_url.as_str())
                    .unwrap_or(t);
                let to_slate_url = slate
                    .transition
                    .as_ref()
                    .and_then(|t| t.to_context.as_ref())
                    .and_then(|tc| tc.slate_context.as_ref())
                    .map(|sc| sc.slate_url.as_str())
                    .unwrap_or(t);

                if from_slate_url.to_string().len() > 0 || to_slate_url.to_string().len() > 0 {
                    log::info!("SLATE MATCHED: from={} to={}", from_slate_url, to_slate_url);
                }
            }
            None => (), //log::debug!("No matches...")
        }

        s
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
    use std::fs::File;
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
        let mut slate =
            File::open("../resources/slate_120px.jpg").expect("Missing file in resources folder");
        let mut buffer = Vec::new();
        slate
            .read_to_end(&mut buffer)
            .expect("Failed to write to buffer");
        let slate = Slate::new(buffer.as_slice(), None).unwrap();
        let detector = SlateDetector::new(vec![slate]).unwrap();
        let slate_img = read_bytes("../resources/slate_120px.jpg");
        let matched_slate = detector.matched_slate(slate_img.as_slice());

        assert!(matched_slate.is_some())
    }

    #[test]
    fn compare_diff_images() {
        let mut slate =
            File::open("../resources/slate_120px.jpg").expect("Missing file in resources folder");
        let mut buffer = Vec::new();
        slate
            .read_to_end(&mut buffer)
            .expect("Failed to write to buffer");
        let slate = Slate::new(buffer.as_slice(), None).unwrap();
        let detector = SlateDetector::new(vec![slate]).unwrap();
        let frame_img = read_bytes("../resources/non-slate_120px.jpg");

        let matched_slate = detector.matched_slate(frame_img.as_slice());

        assert!(matched_slate.is_none())
    }
}
