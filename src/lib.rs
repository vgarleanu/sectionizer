//! Sectionizer
//! This crate contains various utilities useful for detecting similar scenes between video files. This is mostly useful for detecting credits, openings, endings and so on.
//! At the moment only video streams are compared but in the future audio analysis will also be added to augument detection and make it more accurate.
#![feature(box_syntax, slice_group_by)]

pub mod error;

use nightfall::profile::RawVideoProfile;
use nightfall::profile::StreamType;
use nightfall::*;

use futures::join;
use tokio::io::AsyncReadExt;
use tokio::process::ChildStdout;

use std::convert::TryInto;

use bk_tree::BKTree;
use bk_tree::Metric;

const IMG_H: usize = 16;
const IMG_W: usize = 18;
const IMG_SIZE: usize = IMG_H * IMG_W * 3;
const HASHER: img_hash::HashAlg = img_hash::HashAlg::DoubleGradient;
const HASH_MAX_DIST: u64 = 4;
const FRAME_DIST_THRESH: u64 = 5; // 5 seconds

pub type Result<T> = ::core::result::Result<T, crate::error::SectionizerError>;

/// `0` Frame Hash, `1` frame idx
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    hash: u64,
    idx: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct MatchedFrames(Frame, Frame);

/// Metric struct implements `Hamming` distance metric.
pub struct Hamming;

impl Metric<Frame> for Hamming {
    fn distance(&self, a: &Frame, b: &Frame) -> u64 {
        (a.hash ^ b.hash).count_ones() as u64
    }
}

pub struct Sectionizer {
    logger: slog::Logger,
    state: StateManager,
}

impl Sectionizer {
    pub fn new(logger: slog::Logger, state: StateManager) -> Self {
        Self { logger, state }
    }

    /// Method `categorize` attempts to match scenes from `file1` and `file2`, returning the sections which match up.
    /// # Arguments
    /// `file1` - First target file path
    /// `file2` - Second target file path
    ///
    /// # Returns
    /// This method returns a tuple containing two elements, the first field contains the sections for `file1`, the second one contains the sections extracted from `file2`.
    pub async fn categorize<T: ToString>(
        &mut self,
        file1: T,
        file2: T,
    ) -> Result<(Sections, Sections)> {
        let profile = StreamType::RawVideo {
            map: 0,
            profile: RawVideoProfile::RawRgb,
            tt: Some(300),
        };

        let stream1 = self.state.create(profile, file1.to_string()).await?;
        let stream2 = self.state.create(profile, file2.to_string()).await?;

        self.state.start(stream1.clone()).await?;
        self.state.start(stream2.clone()).await?;

        let stream1 = self.state.take_stdout(stream1).await?;
        let stream2 = self.state.take_stdout(stream2).await?;

        // wait for ffmpeg to spit out all the frames for both files.
        let (framevec1, framevec2) = join!(
            self.compute_frame_vec(stream1),
            self.compute_frame_vec(stream2)
        );

        let indextree1 = self.tree_from_vec(framevec1.clone());
        let indextree2 = self.tree_from_vec(framevec2.clone());

        let sections1 = self.get_sections(indextree1, framevec2);
        let sections2 = self.get_sections(indextree2, framevec1);

        Ok((
            Sections {
                target: file1.to_string(),
                sections: sections1,
            },
            Sections {
                target: file2.to_string(),
                sections: sections2,
            },
        ))
    }

    fn get_sections(
        &self,
        indextree: BKTree<Frame, Hamming>,
        framevec: Vec<Frame>,
    ) -> Vec<(u64, u64)> {
        let mut framevec = framevec
            .into_iter()
            .filter_map(|x| indextree.find(&x, HASH_MAX_DIST).next().map(|y| (*y.1, x)))
            .collect::<Vec<_>>();

        // sort framevec to avoid overflow
        framevec.sort_by(|x, y| x.0.idx.cmp(&y.0.idx));

        framevec
            .group_by_mut(|x, y| y.0.idx - x.0.idx < 24 * FRAME_DIST_THRESH)
            .map(|x| {
                x.sort_by(|a, b| a.0.idx.cmp(&b.0.idx));

                let first = x.first().map(|x| x.0.idx).unwrap_or(0);

                x.iter()
                    .map(|x| x.0.idx)
                    .fold((first, 0), |(f, _), x| (f, x))
            })
            .map(|x| (x.0 / 24, x.1 / 24))
            .filter(|x| x.1 - x.0 > 10)
            .collect::<Vec<_>>()
    }

    async fn compute_frame_vec(&self, mut stream: ChildStdout) -> Vec<Frame> {
        let mut frames = Vec::with_capacity(240 * 24);
        let mut buf: Box<[u8; IMG_SIZE]> = box [0; IMG_SIZE];

        let hasher = img_hash::HasherConfig::with_bytes_type::<[u8; 8]>()
            .hash_alg(HASHER)
            .to_hasher();

        let mut idx = 0u64;

        while stream.read_exact(buf.as_mut()).await.is_ok() {
            let raw: &[u8] = buf.as_ref();

            let frame =
                image::RgbImage::from_raw(IMG_W as u32, IMG_H as u32, raw.to_vec()).unwrap();

            let hash = hasher.hash_image(&frame);
            let hash = u64::from_be_bytes(hash.as_bytes().try_into().unwrap());

            let frame = Frame { hash, idx };
            frames.push(frame);
            idx += 1;
        }

        frames
    }

    fn tree_from_vec(&self, frames: Vec<Frame>) -> BKTree<Frame, Hamming> {
        let mut tree = BKTree::new(Hamming);

        for frame in frames {
            tree.add(frame);
        }

        tree
    }
}

pub struct Sections {
    pub target: String,
    pub sections: Vec<(u64, u64)>,
}
