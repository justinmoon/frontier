use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};

static NEXT_TIMER_ID: AtomicU32 = AtomicU32::new(1);

#[derive(Debug, Clone)]
pub enum TimerMessage {
    Fire {
        #[allow(dead_code)]
        timer_id: u32,
    },
}

#[derive(Debug)]
pub struct TimerHandle {
    #[allow(dead_code)]
    pub timer_id: u32,
    cancel_tx: mpsc::UnboundedSender<()>,
}

pub struct TimerRegistry {
    tokio_handle: Handle,
    timers: Arc<Mutex<HashMap<u32, TimerHandle>>>,
    message_tx: mpsc::UnboundedSender<TimerMessage>,
    #[allow(dead_code)]
    message_rx: Arc<Mutex<mpsc::UnboundedReceiver<TimerMessage>>>,
}

impl Default for TimerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerRegistry {
    pub fn new() -> Self {
        let (message_tx, message_rx) = mpsc::unbounded_channel();
        Self {
            tokio_handle: Handle::current(),
            timers: Arc::new(Mutex::new(HashMap::new())),
            message_tx,
            message_rx: Arc::new(Mutex::new(message_rx)),
        }
    }

    pub fn set_timeout(&self, delay_ms: u32) -> u32 {
        let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
        let message_tx = self.message_tx.clone();
        let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();

        self.tokio_handle.spawn(async move {
            tokio::select! {
                _ = sleep(Duration::from_millis(delay_ms as u64)) => {
                    let _ = message_tx.send(TimerMessage::Fire { timer_id });
                }
                _ = cancel_rx.recv() => {
                    // Timer was cancelled
                }
            }
        });

        let handle = TimerHandle {
            timer_id,
            cancel_tx,
        };

        self.timers.lock().unwrap().insert(timer_id, handle);
        timer_id
    }

    pub fn set_interval(&self, delay_ms: u32) -> u32 {
        let timer_id = NEXT_TIMER_ID.fetch_add(1, Ordering::SeqCst);
        let message_tx = self.message_tx.clone();
        let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel();

        self.tokio_handle.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(delay_ms as u64));
            interval.tick().await; // First tick happens immediately, skip it
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if message_tx.send(TimerMessage::Fire { timer_id }).is_err() {
                            break;
                        }
                    }
                    _ = cancel_rx.recv() => {
                        break;
                    }
                }
            }
        });

        let handle = TimerHandle {
            timer_id,
            cancel_tx,
        };

        self.timers.lock().unwrap().insert(timer_id, handle);
        timer_id
    }

    pub fn clear_timer(&self, timer_id: u32) {
        if let Some(handle) = self.timers.lock().unwrap().remove(&timer_id) {
            let _ = handle.cancel_tx.send(());
        }
    }

    #[allow(dead_code)]
    pub fn try_recv_timer(&self) -> Option<TimerMessage> {
        self.message_rx.lock().unwrap().try_recv().ok()
    }

    pub fn clear_all(&self) {
        let mut timers = self.timers.lock().unwrap();
        for (_, handle) in timers.drain() {
            let _ = handle.cancel_tx.send(());
        }
    }
}

impl Drop for TimerRegistry {
    fn drop(&mut self) {
        self.clear_all();
    }
}
