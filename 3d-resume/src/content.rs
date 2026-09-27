//! The resume, baked by `build.rs` from `content/resume.yaml`.

use resume_model::Resume;

const BAKED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/resume.postcard"));

pub fn resume() -> Resume {
    postcard::from_bytes(BAKED).expect("baked resume matches resume-model")
}
