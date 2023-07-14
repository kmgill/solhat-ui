use anyhow::Result;

use gtk::glib::Sender;
use sciimg::prelude::*;
use solhat::calibrationframe::CalibrationImage;
use solhat::context::ProcessContext;
use solhat::threshtest::compute_rgb_threshtest_image;

use crate::state::build_solhat_parameters;
use crate::taskstatus::*;

///////////////////////////////////////////////////////
/// Threshold Testing
///////////////////////////////////////////////////////

pub fn run_thresh_test(master_sender: Sender<TaskStatusContainer>) -> Result<Image> {
    set_task_status(&master_sender, "Processing Threshold Test", 2, 1);
    let context = ProcessContext::create_with_calibration_frames(
        &build_solhat_parameters()?,
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
    )?;

    let first_frame = context.frame_records[0].get_frame(&context)?;
    let result = compute_rgb_threshtest_image(
        &first_frame.buffer,
        context.parameters.obj_detection_threshold as f32,
    );

    set_task_completed(&master_sender);
    Ok(result)
}
