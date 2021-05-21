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

use std::collections::HashMap;
use std::convert::TryInto;

use bktree::BkTree;

const IMG_H: usize = 16;
const IMG_W: usize = 18;
const IMG_SIZE: usize = IMG_H * IMG_W * 3;
const HASHER: img_hash::HashAlg = img_hash::HashAlg::DoubleGradient;
const HASH_MAX_DIST: isize = 2;

pub type Result<T> = ::core::result::Result<T, crate::error::SectionizerError>;

/// `0` Frame Hash, `1` frame idx
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    hash: u128,
    idx: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct MatchedFrames(Frame, Frame);

pub struct Sectionizer {
    #[allow(dead_code)]
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
        reverse: bool,
    ) -> Result<(Sections, Sections)> {
        let sseof = if reverse { Some(300) } else { None };

        let profile = StreamType::RawVideo {
            map: 0,
            profile: RawVideoProfile::RawRgb,
            tt: Some(300),
            sseof,
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

        let sections1 = self.get_sections(indextree2, framevec1);
        let sections2 = self.get_sections(indextree1, framevec2);

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

    fn get_sections(&self, indextree: BkTree<Frame>, framevec: Vec<Frame>) -> Vec<(u128, u128)> {
        let mut framevec = framevec
            .into_iter()
            .filter_map(|x| indextree.find(x, HASH_MAX_DIST).first().map(|y| (x, *y.0)))
            .collect::<Vec<_>>();

        // sort framevec to avoid overflow
        framevec.sort_by(|x, y| x.0.idx.cmp(&y.0.idx));

        let mut groups: HashMap<u64, Vec<Frame>> = HashMap::new();

        for frame in framevec {
            // assumes fps is 24
            let baseframe_idx = frame.0.idx - (frame.0.idx % 24);
            groups.entry(baseframe_idx / 24).or_default().push(frame.0);
        }

        let mut groups = groups
            .into_iter()
            .filter(|(_, x)| x.len() > 1)
            .collect::<Vec<_>>();

        groups.sort_by(|a, b| a.0.cmp(&b.0));

        groups
            .group_by_mut(|(a, _), (b, _)| b - a <= 5)
            .map(|x| {
                x.sort_by_key(|(a, _)| *a);

                let first = x.first().map(|(x, _)| *x).unwrap_or(0);

                x.iter()
                    .map(|(x, _)| *x)
                    .fold((first, 0), |(f, _), x| (f, x))
            })
            .map(|x| (x.0 as u128, x.1 as u128))
            .collect::<Vec<_>>()
    }

    async fn compute_frame_vec(&self, mut stream: ChildStdout) -> Vec<Frame> {
        let mut frames = Vec::with_capacity(240 * 24);
        let mut buf: Box<[u8; IMG_SIZE]> = box [0; IMG_SIZE];

        let hasher = img_hash::HasherConfig::with_bytes_type::<[u8; 16]>()
            .hash_alg(HASHER)
            .preproc_dct()
            .to_hasher();

        let mut idx = 0u64;

        while stream.read_exact(buf.as_mut()).await.is_ok() {
            let raw: &[u8] = buf.as_ref();

            let frame =
                image::RgbImage::from_raw(IMG_W as u32, IMG_H as u32, raw.to_vec()).unwrap();

            let hash = hasher.hash_image(&frame);
            let hash = u128::from_be_bytes(hash.as_bytes().try_into().unwrap());

            let frame = Frame { hash, idx };
            frames.push(frame);
            idx += 1;
        }

        frames
    }

    fn tree_from_vec(&self, frames: Vec<Frame>) -> BkTree<Frame> {
        let mut tree = BkTree::new(hamming);
        tree.insert_all(frames);

        tree
    }
}

pub struct Sections {
    pub target: String,
    pub sections: Vec<(u128, u128)>,
}

pub fn hamming(a: &Frame, b: &Frame) -> isize {
    (a.hash ^ b.hash).count_ones() as isize
}
