use anyhow::Result;
use gtk::glib::Sender;
// use solhat::anaysis::frame_sigma_analysis_window_size;
use solhat::context::ProcessContext;
use solhat::drizzle::BilinearDrizzle;
use solhat::framerecord::FrameRecord;
use solhat::limiting::frame_limit_determinate;
// use solhat::offsetting::frame_offset_analysis;
use solhat::rotation::frame_rotation_analysis;
use solhat::stacking::process_frame_stacking;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cancel::*;
use crate::state::*;
use crate::taskstatus::*;
use crate::analysis::sigma::frame_analysis_window_size;

pub async fn run_async(
    master_sender: Sender<TaskStatusContainer>,
    output_filename: PathBuf,
) -> Result<()> {
    info!("Async task started");

    let mut context = build_solhat_context(&master_sender)?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    context.frame_records = frame_sigma_analysis(&context, master_sender.clone())?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    context.frame_records = frame_limiting(&context, master_sender.clone())?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    context.frame_records = frame_rotation(&context, master_sender.clone())?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    if context.frame_records.is_empty() {
        println!("Zero frames to stack. Cannot continue");
    } else {
        let drizzle_output = drizzle_stacking(&context, master_sender.clone())?;

        check_cancel_status(&master_sender)?;
        set_task_status(&master_sender, "Finalizing", 0, 0);
        let mut stacked_buffer = drizzle_output.get_finalized().unwrap();

        // Let the user know some stuff...
        let (stackmin, stackmax) = stacked_buffer.get_min_max_all_channel();
        info!(
            "    Stack Min/Max : {}, {} ({} images)",
            stackmin,
            stackmax,
            context.frame_records.len()
        );

        if get_state_param!(decorrelated_colors) {
            stacked_buffer.normalize_to_16bit_decorrelated();
        } else {
            stacked_buffer.normalize_to_16bit();
        }

        info!(
            "Final image size: {}, {}",
            stacked_buffer.width, stacked_buffer.height
        );

        // Save finalized image to disk
        set_task_status(&master_sender, "Saving", 0, 0);
        stacked_buffer.save(output_filename.to_string_lossy().as_ref())?;

        // The user will likely never see this actually appear on screen
        set_task_status(&master_sender, "Done", 1, 1);
    }

    set_task_completed(&master_sender);

    Ok(())
}


fn frame_sigma_analysis(
    context: &ProcessContext,
    sender: Sender<TaskStatusContainer>,
) -> Result<Vec<FrameRecord>> {
    check_cancel_status(&sender)?;

    let frame_count = context.frame_records.len();

    set_task_status(&sender, "Frame Analysis", frame_count, 0);

    let counter = Arc::new(Mutex::new(0));

    let frame_records = frame_analysis_window_size(
        context,
        context.parameters.analysis_window_size,
        move |fr| {
            info!(
                "frame_sigma_analysis(): Frame processed with sigma {}",
                fr.sigma
            );
            // check_cancel_status(&sender);

            let mut c = counter.lock().unwrap();
            *c += 1;
            set_task_status(&sender, "Frame Analysis", frame_count, *c)
        },
    )?;

    Ok(frame_records)
}

fn frame_limiting(
    context: &ProcessContext,
    sender: Sender<TaskStatusContainer>,
) -> Result<Vec<FrameRecord>> {
    check_cancel_status(&sender)?;

    let frame_count = context.frame_records.len();

    set_task_status(&sender, "Applying Frame Limits", frame_count, 0);

    let counter = Arc::new(Mutex::new(0));

    let frame_records = frame_limit_determinate(context, move |_fr| {
        info!("frame_limit_determinate(): Frame processed.");
        // check_cancel_status(&sender);

        let mut c = counter.lock().unwrap();
        *c += 1;
        set_task_status(&sender, "Applying Frame Limits", frame_count, *c)
    })?;

    Ok(frame_records)
}

fn frame_rotation(
    context: &ProcessContext,
    sender: Sender<TaskStatusContainer>,
) -> Result<Vec<FrameRecord>> {
    check_cancel_status(&sender)?;

    let frame_count = context.frame_records.len();

    set_task_status(
        &sender,
        "Computing Parallactic Angle Rotations",
        frame_count,
        0,
    );

    let counter = Arc::new(Mutex::new(0));

    let frame_records = frame_rotation_analysis(context, move |fr| {
        info!(
            "Rotation for frame is {} degrees",
            fr.computed_rotation.to_degrees()
        );
        // check_cancel_status(&sender);

        let mut c = counter.lock().unwrap();
        *c += 1;
        set_task_status(
            &sender,
            "Computing Parallactic Angle Rotations",
            frame_count,
            *c,
        )
    })?;

    Ok(frame_records)
}

fn drizzle_stacking(
    context: &ProcessContext,
    sender: Sender<TaskStatusContainer>,
) -> Result<BilinearDrizzle> {
    check_cancel_status(&sender)?;

    let frame_count = context.frame_records.len();

    set_task_status(&sender, "Stacking", frame_count, 0);

    let counter = Arc::new(Mutex::new(0));

    let drizzle_output = process_frame_stacking(context, move |_fr| {
        info!("process_frame_stacking(): Frame processed.");
        // check_cancel_status(&sender);

        let mut c = counter.lock().unwrap();
        *c += 1;
        set_task_status(&sender, "Stacking", frame_count, *c)
    })?;

    Ok(drizzle_output)
}
