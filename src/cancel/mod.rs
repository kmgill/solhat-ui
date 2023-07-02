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

macro_rules! set_request_cancel {
    () => {
        cancel::CANCEL_TASK.lock().unwrap().status = CancelStatus::CancelRequested;
    };
}

macro_rules! set_task_cancelled {
    () => {
        cancel::CANCEL_TASK.lock().unwrap().status = CancelStatus::Cancelled;
    };
}

macro_rules! reset_cancel_status {
    () => {
        cancel::CANCEL_TASK.lock().unwrap().status = CancelStatus::NoStatus;
    };
}

macro_rules! is_cancel_requested {
    () => {
        cancel::CANCEL_TASK.lock().unwrap().status == CancelStatus::CancelRequested
    };
}
