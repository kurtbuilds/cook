mod file;
pub use file::api::*;

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
