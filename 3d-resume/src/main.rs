//! The 3D resume's program (desktop, iOS and web); everything lives in the
//! library, which Android loads directly.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    resume_3d::run();
}
