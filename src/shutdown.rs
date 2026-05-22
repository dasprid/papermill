use std::sync::atomic::{AtomicBool, Ordering};

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

pub fn install_handler() {
    tokio::spawn(async {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("Received interrupt. Finishing current invoice and exiting…");
            SHUTDOWN.store(true, Ordering::Relaxed);
        }
    });
}

pub fn is_shutting_down() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}
