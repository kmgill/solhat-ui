use anyhow::Result;
use gtk::glib::Sender;
use solhat::anaysis::frame_sigma_analysis_window_size;
use solhat::limiting::frame_limit_determinate;
use solhat::offsetting::frame_offset_analysis;
use solhat::rotation::frame_rotation_analysis;
use solhat::stacking::process_frame_stacking;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cancel::*;
use crate::state::*;
use crate::taskstatus::*;

lazy_static! {
    // NOTE: Concurrent processing threads will stomp on each other, but at least
    // they'll do it in proper turn.  Also, this is stupid and can't stay this way.
    static ref COUNTER: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
}

pub async fn run_async(
    master_sender: Sender<TaskStatusContainer>,
    output_filename: PathBuf,
) -> Result<()> {
    info!("Async task started");

    // let output_filename = assemble_output_filename()?;
    // let params = build_solhat_parameters();
    let mut context = build_solhat_context(&master_sender)?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    check_cancel_status(&master_sender)?;
    let frame_count = context.frame_records.len();
    let sender = master_sender.clone();
    set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, 0);
    context.frame_records = frame_offset_analysis(&context, move |_fr| {
        info!("frame_offset_analysis(): Frame processed.");
        // check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c += 1;
        set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, *c)
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////
    check_cancel_status(&master_sender)?;
    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    set_task_status(&sender, "Frame Sigma Analysis", frame_count, 0);
    context.frame_records = frame_sigma_analysis_window_size(
        &context,
        context.parameters.analysis_window_size,
        move |fr| {
            info!(
                "frame_sigma_analysis(): Frame processed with sigma {}",
                fr.sigma
            );
            // check_cancel_status(&sender);

            let mut c = COUNTER.lock().unwrap();
            *c += 1;
            set_task_status(&sender, "Frame Sigma Analysis", frame_count, *c)
        },
    )?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    check_cancel_status(&master_sender)?;
    set_task_status(&sender, "Applying Frame Limits", frame_count, 0);
    context.frame_records = frame_limit_determinate(&context, move |_fr| {
        info!("frame_limit_determinate(): Frame processed.");
        // check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c += 1;
        set_task_status(&sender, "Applying Frame Limits", frame_count, *c)
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    check_cancel_status(&master_sender)?;
    set_task_status(
        &sender,
        "Computing Parallactic Angle Rotations",
        frame_count,
        0,
    );
    context.frame_records = frame_rotation_analysis(&context, move |fr| {
        info!(
            "Rotation for frame is {} degrees",
            fr.computed_rotation.to_degrees()
        );
        // check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c += 1;
        set_task_status(
            &sender,
            "Computing Parallactic Angle Rotations",
            frame_count,
            *c,
        )
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    if context.frame_records.is_empty() {
        println!("Zero frames to stack. Cannot continue");
    } else {
        let frame_count = context.frame_records.len();
        *COUNTER.lock().unwrap() = 0;
        let sender = master_sender.clone();
        check_cancel_status(&master_sender)?;
        set_task_status(&sender, "Stacking", frame_count, 0);
        let drizzle_output = process_frame_stacking(&context, move |_fr| {
            info!("process_frame_stacking(): Frame processed.");
            // check_cancel_status(&sender);

            let mut c = COUNTER.lock().unwrap();
            *c += 1;
            set_task_status(&sender, "Stacking", frame_count, *c)
        })?;

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
