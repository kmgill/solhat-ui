use std::sync::{Arc, Mutex};

#[derive(Debug, PartialEq, Eq)]
pub enum CancelStatus {
    NoStatus,        // Keep doing what you're doing...
    CancelRequested, // Request cancel
    Cancelled,       // Task has cancelled
}

pub struct CancelContainer {
    pub status: CancelStatus,
}

lazy_static! {
    pub static ref CANCEL_TASK: Arc<Mutex<CancelContainer>> =
        Arc::new(Mutex::new(CancelContainer {
            status: CancelStatus::NoStatus
        }));
}

pub fn set_request_cancel() {
    CANCEL_TASK.lock().unwrap().status = CancelStatus::CancelRequested;
}

pub fn set_task_cancelled() {
    CANCEL_TASK.lock().unwrap().status = CancelStatus::Cancelled;
}

pub fn reset_cancel_status() {
    CANCEL_TASK.lock().unwrap().status = CancelStatus::NoStatus;
}

pub fn is_cancel_requested() -> bool {
    CANCEL_TASK.lock().unwrap().status == CancelStatus::CancelRequested
}
