mod cargo;
mod file;
mod ghrelease;
mod package;
mod user;
mod which;
use std::sync::Mutex;

pub use file::api::*;
pub use ghrelease::api::*;
pub use package::api::*;
pub use user::api::*;
pub use user::api::*;
pub use which::api::*;

extern "C" fn cleanup() {
    let files = file::spec::FILE_SPEC.lock().unwrap();
    for file in files.iter() {
        dbg!(file);
    }
}

#[ctor::ctor]
fn register_at_exit() {
    unsafe {
        let result = libc::atexit(cleanup);
        if result != 0 {
            eprintln!("Failed to register atexit handler");
        }
    }
}

pub(crate) static CHECKS: Mutex<Vec<Box<dyn Check + Send + Sync + 'static>>> =
    Mutex::new(Vec::new());

pub trait Check {
    fn check(&self) -> Result<Vec<Box<dyn Apply>>, Error>;
    fn check_ssh(&self) -> Result<Vec<Box<dyn Apply>>, Error>;
}

pub trait Apply {
    fn apply(&self) -> Result<(), Error>;
    fn apply_ssh(&self) -> Result<(), Error>;
}

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
