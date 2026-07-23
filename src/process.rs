use std::process::{Child, ExitStatus};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) if start.elapsed() < timeout => sleep(Duration::from_millis(100)),
            _ => return None,
        }
    }
}
