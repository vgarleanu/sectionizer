#![feature(box_syntax, slice_group_by)]

use nightfall::profile::RawVideoProfile;
use nightfall::profile::StreamType;
use nightfall::*;

use slog::o;
use slog::Drain;

use futures::io::AllowStdIo;
use futures::io::AsyncReadExt;
use futures::io::BufReader;
use futures::join;

use std::convert::TryInto;
use std::path::Path;
use std::process::ChildStdout;

use bk_tree::BKTree;
use bk_tree::Metric;

/// `0` Frame Hash, `1` frame idx
#[derive(Clone, Copy)]
pub struct Frame(u64, u64);

#[derive(Clone, Copy)]
pub struct MatchedFrames(Frame, Frame);

pub struct Hamming;

impl Metric<Frame> for Hamming {
    fn distance(&self, a: &Frame, b: &Frame) -> u64 {
        (a.0 ^ b.0).count_ones() as u64
    }
}

pub async fn get_chapters<T: ToString>(
    state: StateManager,
    logger: slog::Logger,
    file1: T,
    file2: T,
) -> Vec<(u64, u64)> {
    let stream = state
        .create(
            StreamType::RawVideo {
                map: 0,
                profile: RawVideoProfile::RawRgb,
                tt: Some(240),
            },
            file1.to_string()
        )
        .await
        .unwrap();

    state.start(stream.clone()).await.unwrap();

    let stdout1 = state.take_stdout(stream.clone()).await.unwrap();

    let stream2 = state
        .create(
            StreamType::RawVideo {
                map: 0,
                profile: RawVideoProfile::RawRgb,
                tt: Some(240),
            },
            file2.to_string()
        )
        .await
        .unwrap();

    state.start(stream2.clone()).await.unwrap();
    let stdout2 = state.take_stdout(stream2.clone()).await.unwrap();

    let (base_tree, frame_vec) = join!(tree_from_stdout(stdout1), vec_from_stdout(stdout2));
    let frame_vec_len = frame_vec.len();

    let mut matched_frames = Vec::new();

    const HASH_DIST: u64 = 3;
    for frame in frame_vec {
        let matches = base_tree.find(&frame, HASH_DIST).collect::<Vec<_>>();
        if let Some(x) = matches.first() {
            matched_frames.push(MatchedFrames(*x.1, frame));
        }
    }

    matched_frames.sort_by(|x, y| x.0 .1.cmp(&y.0 .1));

    const FRAME_DIST_THRESH: u64 = 5; // 5 seconds

    matched_frames
        .group_by_mut(|x, y| y.0 .1 - x.0 .1 < 24 * FRAME_DIST_THRESH)
        .filter_map(|x| {
            x.iter_mut().reduce(|x, y| {
                x.1 = y.0;
                x
            })
        })
        // filter out sections less than 10 seconds in size
        .filter(|x| x.1 .1 - x.0 .1 > 24 * 10)
        .map(|x| (x.0.1 / 24, x.1.1 / 24))
        .collect::<Vec<_>>()


}

async fn tree_from_stdout(stdout: ChildStdout) -> BKTree<Frame, Hamming> {
    let mut tree = BKTree::new(Hamming);
    let mut buf: Box<[u8; 8 * 8 * 3]> = box [0; 8 * 8 * 3];
    let mut stdout = BufReader::with_capacity(8 * 8 * 3, AllowStdIo::new(stdout));

    let hasher = img_hash::HasherConfig::new()
        .hash_alg(img_hash::HashAlg::Blockhash)
        .to_hasher();

    let mut idx = 0;

    while stdout.read_exact(buf.as_mut()).await.is_ok() {
        let raw: &[u8] = buf.as_ref();

        let frame = image::RgbImage::from_raw(8, 8, raw.to_vec()).unwrap();

        let hash = hasher.hash_image(&frame);
        let hash = u64::from_be_bytes(hash.as_bytes().try_into().unwrap());

        let frame = Frame(hash, idx);
        tree.add(frame);
        idx += 1;
    }

    tree
}

async fn vec_from_stdout(stdout: ChildStdout) -> Vec<Frame> {
    let mut frames = Vec::with_capacity(240 * 24);
    let mut buf: Box<[u8; 8 * 8 * 3]> = box [0; 8 * 8 * 3];
    let mut stdout = BufReader::with_capacity(8 * 8 * 3, AllowStdIo::new(stdout));

    let hasher = img_hash::HasherConfig::new()
        .hash_alg(img_hash::HashAlg::Blockhash)
        .to_hasher();

    let mut idx = 0;

    while stdout.read_exact(buf.as_mut()).await.is_ok() {
        let raw: &[u8] = buf.as_ref();

        let frame = image::RgbImage::from_raw(8, 8, raw.to_vec()).unwrap();

        let hash = hasher.hash_image(&frame);
        let hash = u64::from_be_bytes(hash.as_bytes().try_into().unwrap());

        let frame = Frame(hash, idx);
        frames.push(frame);
        idx += 1;
    }

    frames
}
