#[macro_use]
mod state;
use gtk::glib::Type;
use state::*;

#[macro_use]
mod cancel;
use cancel::*;

#[macro_use]
mod taskstatus;
use taskstatus::*;

use anyhow::Result;
use gtk::gdk::Display;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf, PixbufLoader};
#[allow(deprecated)]
use gtk::{
    gio, prelude::*, Adjustment, ComboBoxText, CssProvider, Entry, Label, Picture, ProgressBar,
    ScrolledWindow, SpinButton, TextBuffer, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use gtk::{glib, Application, ApplicationWindow, Builder, Button};
use itertools::iproduct;
use queues::IsQueue;
use queues::{queue, Queue};
use solhat::anaysis::frame_sigma_analysis;
use solhat::calibrationframe::{CalibrationImage, ComputeMethod};
use solhat::context::{ProcessContext, ProcessParameters};
use solhat::drizzle::Scale;
use solhat::limiting::frame_limit_determinate;
use solhat::offsetting::frame_offset_analysis;
use solhat::rotation::frame_rotation_analysis;
use solhat::ser::{SerFile, SerFrame};
use solhat::stacking::process_frame_stacking;
use solhat::target::Target;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

#[macro_use]
extern crate stump;

#[macro_use]
extern crate lazy_static;

#[derive(Debug)]
pub struct LogQueue {
    q: Queue<String>,
}

lazy_static! {
    static ref LOG_QUEUE: Arc<Mutex<LogQueue>> = Arc::new(Mutex::new(LogQueue { q: queue![] }));
}

#[tokio::main]
async fn main() -> Result<glib::ExitCode> {
    stump::set_min_log_level(stump::LogEntryLevel::DEBUG);
    info!("Starting SolHat-UI");

    let application = gtk::Application::new(Some("com.apoapsys.solhat"), Default::default());

    application.connect_startup(|_| {
        // The CSS "magic" happens here.
        let provider = CssProvider::new();
        provider.load_from_data(include_str!("../assets/styles.css"));

        let display = Display::default().expect("Could not connect to a display.");

        // We give the CssProvided to the default screen so the CSS rules we added
        // can be applied to our window.
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let icon_theme = gtk::IconTheme::for_display(&display);
        icon_theme.add_search_path("assets/");
        // Note: The icon has been found, but still not being used by the window.
        // The icon-name property has been set in the template.
        if !icon_theme.has_icon("solhat") {
            warn!("SolHat Icon Not Found!");
        }

        // We build the application UI.
        // build_ui(app);
    });

    application.connect_activate(build_ui);
    let exitcode = application.run();

    STATE.lock().unwrap().save_to_userhome()?;
    Ok(exitcode)
}

macro_rules! bind_object {
    ($builder:expr, $obj_id:expr) => {
        if let Some(obj) = $builder.object($obj_id) {
            obj
        } else {
            panic!("Failed to bind object with id '{}'", $obj_id);
        }
    };
}

macro_rules! update_output_filename {
    ($builder:expr) => {
        let lbl_output_filename: Label = bind_object!($builder, "lbl_output_filename");
        lbl_output_filename.set_label(
            assemble_output_filename()
                .unwrap()
                .file_name()
                .unwrap()
                .to_str()
                .unwrap(),
        );
    };
}

macro_rules! update_preview_from_ser_file {
    ($builder:expr,$ser_file_path:expr, $preview_id:expr) => {
        let pic: Picture = bind_object!($builder, $preview_id);

        if let Some(ext) = $ser_file_path.extension() {
            match ext.to_str().unwrap() {
                "ser" => {
                    let pix = picture_from_ser_file($ser_file_path.to_str().unwrap()).unwrap();
                    pic.set_pixbuf(Some(&pix));
                }
                _ => {
                    error!("User loaded an invalid ser file: {:?}", $ser_file_path);
                    // Load an 'invalid file' icon
                }
            }
        }
    };
}

/// Binds the controls for the input files. The controls are the label, open, and clear buttons
macro_rules! bind_open_clear {
    ($builder:expr, $window:expr, $open_id:expr, $clear_id:expr, $label_id:expr, $preview_id:expr, $state_prop:ident, $opener:ident) => {{
        let btn_open: Button = bind_object!($builder, $open_id);
        let btn_clear: Button = bind_object!($builder, $clear_id);
        let label: Label = bind_object!($builder, $label_id);

        if let Some(prop) = &STATE.lock().unwrap().params.$state_prop {
            label.set_label(prop.file_name().unwrap().clone().to_str().unwrap());
        }

        let win = &$window;

        let b = $builder.clone();
        btn_open.connect_clicked(glib::clone!(@strong label, @weak win, @weak b as builder => move |_| {
            debug!("Opening file");
            $opener("Open Ser File", &win,glib::clone!( @weak label => move|f| {
                debug!("Opened: {:?}", f);
                label.set_label(f.file_name().unwrap().to_str().unwrap());
                set_state_param!($state_prop, Some(f.to_owned()));
                update_output_filename!(builder);
                set_last_opened_folder!(f.parent().unwrap().to_owned());
                update_execute_state!(builder);
                // We'll update regardless of which input data was opened as the goal
                // is to provide a minimal calibration as data is identified prior to showing
                // the preview to the user
                if !$preview_id.is_empty()  {
                    update_preview_from_ser_file!(builder, f, $preview_id);
                }

            }));
        }));

        let b = $builder.clone();
        btn_clear.connect_clicked(glib::clone!(@strong label, @weak b as builder => move |_| {
            label.set_label("");
            set_state_param!($state_prop, None);
            update_execute_state!(builder);
            update_output_filename!(builder);
        }));
    }};
}

macro_rules! bind_spinner {
    ($builder:expr, $obj_id:expr, $state_prop:ident, $type:ident) => {
        let spn_obj: SpinButton = bind_object!($builder, $obj_id);
        let spn_adj: Adjustment = spn_obj.adjustment();
        spn_adj.set_value(get_state_param!($state_prop) as f64);
        spn_adj.connect_value_changed(|e| {
            info!("Spinner with id {} set to {}", $obj_id, e.value());
            set_state_param!($state_prop, e.value() as $type);
            info!("State param is now {}", get_state_param!($state_prop));
        });
    };
}

macro_rules! set_execute_enabled {
    ($builder:expr,$enabled:expr) => {
        let start: Button = bind_object!($builder, "btn_execute");
        start.set_sensitive($enabled);
    };
}

macro_rules! update_execute_state {
    ($builder:expr) => {
        set_execute_enabled!(
            $builder,
            get_state_param!(light).is_some() && get_state_param!(output_dir).is_some()
        );
    };
}

#[allow(deprecated)]
fn build_ui(application: &Application) {
    let ui_src = include_str!("../assets/solhat.ui");
    let builder = Builder::from_string(ui_src);

    if let Ok(ss) = ApplicationState::load_from_userhome() {
        *STATE.lock().unwrap() = ss;
    } else {
        warn!("No saved state file found. One will be created on exit");
    }

    let window: ApplicationWindow = builder
        .object("SolHatApplicationMain")
        .expect("Couldn't get window");
    window.set_application(Some(application));
    // window.set_icon_name(Some("solhat"));
    // window.set_icon(Some(&loader.pixbuf().unwrap()));
    // window.icon

    bind_open_clear!(
        builder,
        window,
        "btn_light_open",
        "btn_light_clear",
        "lbl_light",
        "img_preview_light",
        light,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_dark_open",
        "btn_dark_clear",
        "lbl_dark",
        "img_preview_dark",
        dark,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_flat_open",
        "btn_flat_clear",
        "lbl_flat",
        "img_preview_flat",
        flat,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_darkflat_open",
        "btn_darkflat_clear",
        "lbl_darkflat",
        "img_preview_darkflat",
        darkflat,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_bias_open",
        "btn_bias_clear",
        "lbl_bias",
        "img_preview_bias",
        bias,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_hotpixelmap_open",
        "btn_hotpixelmap_clear",
        "lbl_hotpixelmap",
        "",
        hot_pixel_map,
        open_toml_file
    );

    ////////
    // Output folder
    ////////

    let btn_output_open: Button = bind_object!(builder, "btn_output_folder_open");
    let lbl_output_folder: Label = bind_object!(builder, "lbl_output_folder");

    if let Some(prop) = &STATE.lock().unwrap().params.output_dir {
        lbl_output_folder.set_label(prop.clone().to_str().unwrap());
    }

    let b = builder.clone();
    btn_output_open.connect_clicked(
        glib::clone!(@strong lbl_output_folder, @weak window => move |_| {
            debug!("Opening file");
            open_folder("Open Ser File", &window,glib::clone!( @weak lbl_output_folder, @weak b as builder => move|f| {
                debug!("Opened: {:?}", f);
                lbl_output_folder.set_label(f.to_str().unwrap());
                set_state_param!(output_dir, Some(f.to_owned()));
                update_output_filename!(builder);
                set_last_opened_folder!(f.to_owned());
            }));
        }),
    );

    ////////
    // Free text
    ////////
    let b = builder.clone();
    let txt_freetext: Entry = bind_object!(builder, "txt_freetext");
    txt_freetext.set_text(&get_state_param!(freetext));
    txt_freetext.connect_changed(glib::clone!(@weak window, @weak b as builder => move |e| {
        debug!("Free Text: {}", e.buffer().text());
        set_state_param!(freetext, e.buffer().text().to_string());
        update_output_filename!(builder);
    }));

    ////////
    // Target
    ////////

    let combo_target: ComboBoxText = bind_object!(builder, "combo_target");
    match STATE.lock().unwrap().params.target {
        Target::Sun => combo_target.set_active_id(Some("0")),
        Target::Moon => combo_target.set_active_id(Some("1")),
    };
    combo_target.connect_changed(glib::clone!(@weak window, @weak b as builder => move |e| {
        set_state_param!(target, match e.active_id().unwrap().to_string().as_str() {
            "0" => Target::Sun,
            "1" => Target::Moon,
            _ => panic!("Invalid target selected")
        });
        update_output_filename!(builder);
    }));

    ////////
    // Drizzle
    ////////
    let combo_drizzle: ComboBoxText = bind_object!(builder, "combo_drizzle");
    match STATE.lock().unwrap().params.drizzle_scale {
        Scale::Scale1_0 => combo_drizzle.set_active_id(Some("0")),
        Scale::Scale1_5 => combo_drizzle.set_active_id(Some("1")),
        Scale::Scale2_0 => combo_drizzle.set_active_id(Some("2")),
        Scale::Scale3_0 => combo_drizzle.set_active_id(Some("3")),
    };
    combo_drizzle.connect_changed(glib::clone!(@weak window, @weak b as builder => move |e| {
        set_state_param!(drizzle_scale, match e.active_id().unwrap().to_string().as_str() {
            "0" => Scale::Scale1_0,
            "1" => Scale::Scale1_5,
            "2" => Scale::Scale2_0,
            "3" => Scale::Scale3_0,
            _ => panic!("Invalid drizzle scale selected")
        });
        update_output_filename!(builder);
    }));

    ////////
    // Spinners
    ////////
    bind_spinner!(builder, "spn_obs_latitude", obs_latitude, f64);
    bind_spinner!(builder, "spn_obs_longitude", obs_longitude, f64);

    bind_spinner!(
        builder,
        "spn_obj_detection_threshold",
        obj_detection_threshold,
        f64
    );
    bind_spinner!(builder, "spn_max_frames", max_frames, usize);
    bind_spinner!(builder, "spn_min_sigma", min_sigma, f64);
    bind_spinner!(builder, "spn_max_sigma", max_sigma, f64);
    bind_spinner!(builder, "spn_top_percentage", top_percentage, f64);

    update_execute_state!(builder);
    // set_execute_enabled!(builder, false);
    let start: Button = bind_object!(builder, "btn_execute");
    let b = builder.clone();
    start.connect_clicked(glib::clone!(@weak window, @weak b as builder => move |_| {
        debug!("Start has been clicked");
        tokio::spawn(async move {
            {
                run_async().await.unwrap(); //.await.unwrap();
            }
        });
    }));

    ////////
    // Task Monitor
    ////////
    // let b = Rc::new(Cell::new(builder.clone()));
    let label: Label = bind_object!(b, "lbl_task_status");
    let progress: ProgressBar = bind_object!(b, "prg_task_progress");
    let cancel: Button = bind_object!(b, "btn_cancel");
    let log_buffer: TextBuffer = bind_object!(builder, "txt_log_buffer");
    let scrl_log_output: ScrolledWindow = bind_object!(builder, "scrl_log_output");
    cancel.connect_clicked(move |_| {
        set_request_cancel!();
    });
    let update_state_callback = glib::clone!(@weak label, @weak start => @default-return Continue(true), move || {
        let proc_status = taskstatus::TASK_STATUS_QUEUE.lock().unwrap();
        match &proc_status.status {
            Some(TaskStatus::TaskPercentage(task_name, len, cnt)) => {
                let pct = if *len > 0 {
                    *cnt as f64 / *len as f64
                } else {
                    0.0
                };
                label.set_visible(true);
                progress.set_visible(true);
                cancel.set_visible(true);
                label.set_label(&task_name);
                progress.set_fraction(pct);
                start.set_sensitive(false);
                cancel.set_sensitive(true);
            },
            None => {
                label.set_visible(false);
                progress.set_visible(false);
                cancel.set_visible(false);
                start.set_sensitive(true);
                cancel.set_sensitive(false);
            }
        };
        // let spn: Spinner = bind_object!(b, "prg_task_progress");

        let mut q = LOG_QUEUE
            .lock()
            .unwrap();

        while q.q.size() > 0 {
            let s = q.q.remove().expect("Failed to remove queue item");
            let mut end = log_buffer.end_iter();
            log_buffer.insert(&mut end, "\n");
            log_buffer.insert(&mut end, &s);

            // Scroll to bottom
            let vadjustment = scrl_log_output.vadjustment();
            vadjustment.set_value(vadjustment.upper());
        }


        Continue(true)
    });

    let _ = glib::timeout_add_local(Duration::from_millis(250), update_state_callback);

    ////////
    // Logging
    ////////
    //
    stump::set_print(|s| {
        LOG_QUEUE
            .lock()
            .unwrap()
            .q
            .add(s.to_owned())
            .expect("Queue add failed");
        println!("{}", s);
    });

    update_output_filename!(builder);

    // If there's a file in the parameters state already (such as from saved state),
    // we need to update the preview pane.
    if let Some(light_path) = &STATE.lock().unwrap().params.light {
        update_preview_from_ser_file!(builder, light_path, "img_preview_light");
    }

    window.present();
}

fn open_ser_file<F>(title: &str, window: &ApplicationWindow, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    open_file(title, window, "video/ser", "SER", callback);
}

fn open_toml_file<F>(title: &str, window: &ApplicationWindow, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    open_file(title, window, "application/toml", "toml", callback);
}

fn open_file<F>(
    title: &str,
    window: &ApplicationWindow,
    mimetype: &str,
    mimename: &str,
    callback: F,
) where
    F: Fn(PathBuf) + 'static,
{
    let filters = gio::ListStore::new(Type::OBJECT);
    let ser_filter = gtk::FileFilter::new();
    ser_filter.add_mime_type(mimetype);
    ser_filter.set_name(Some(mimename));
    filters.append(&ser_filter);

    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Open")
        .modal(true)
        .filters(&filters)
        .build();

    dialog.open(Some(window), gio::Cancellable::NONE, move |file| {
        if let Ok(file) = file {
            let filename = file.path().expect("Couldn't get file path");
            callback(filename);
        }
    });
}

fn open_folder<F>(title: &str, window: &ApplicationWindow, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Open")
        .modal(true)
        .build();
    dialog.select_folder(Some(window), gio::Cancellable::NONE, move |file| {
        if let Ok(file) = file {
            let filename = file.path().expect("Couldn't get file path");

            callback(filename);
        }
    });
}

fn assemble_output_filename() -> Result<PathBuf> {
    let state = STATE.lock().unwrap();

    let output_dir = if let Some(output_dir) = &state.params.output_dir {
        output_dir.to_owned()
    } else {
        dirs::home_dir().unwrap()
    };

    let base_filename = if let Some(input_file) = &state.params.light {
        Path::new(input_file.file_name().unwrap())
            .file_stem()
            .unwrap()
    } else {
        OsStr::new("Unknown")
    };

    let freetext = if !state.params.freetext.is_empty() {
        format!("_{}", state.params.freetext)
    } else {
        "".to_owned()
    };

    let drizzle = match state.params.drizzle_scale {
        Scale::Scale1_0 => "".to_owned(),
        _ => format!(
            "_{}",
            state
                .params
                .drizzle_scale
                .to_string()
                .replace([' ', '.'], "")
        ),
    };

    let output_filename = format!(
        "{}_{:?}{}{}.tif",
        base_filename.to_string_lossy().as_ref(),
        state.params.target,
        drizzle,
        freetext
    );
    let output_path: PathBuf = Path::new(&output_dir).join(output_filename);
    Ok(output_path)
}

fn picture_from_ser_file(file_path: &str) -> Result<Pixbuf> {
    let ser_file = SerFile::load_ser(file_path).unwrap();
    let first_image = ser_file.get_frame(0).unwrap();
    ser_frame_to_picture(&first_image)
}

fn ser_frame_to_picture(ser_frame: &SerFrame) -> Result<Pixbuf> {
    let mut copied = ser_frame.buffer.clone();

    copied.normalize_to_8bit();

    let pix = Pixbuf::new(
        Colorspace::Rgb,
        false,
        8,
        copied.width as i32,
        copied.height as i32,
    )
    .unwrap();

    iproduct!(0..copied.height, 0..copied.width).for_each(|(y, x)| {
        let (r, g, b) = if copied.num_bands() == 1 {
            (
                copied.get_band(0).get(x, y),
                copied.get_band(0).get(x, y),
                copied.get_band(0).get(x, y),
            )
        } else {
            (
                copied.get_band(0).get(x, y),
                copied.get_band(1).get(x, y),
                copied.get_band(2).get(x, y),
            )
        };
        pix.put_pixel(x as u32, y as u32, r as u8, g as u8, b as u8, 255);
    });
    Ok(pix)
}

macro_rules! p2s {
    ($pb:expr) => {
        if let Some(pb) = &$pb {
            Some(pb.as_os_str().to_str().unwrap().to_string().to_owned())
        } else {
            None
        }
    };
}

fn build_solhat_parameters() -> ProcessParameters {
    let state = STATE.lock().unwrap();

    ProcessParameters {
        input_files: vec![p2s!(state.params.light).unwrap()],
        obj_detection_threshold: state.params.obj_detection_threshold,
        obs_latitude: state.params.obs_latitude,
        obs_longitude: state.params.obs_longitude,
        target: state.params.target,
        crop_width: None,
        crop_height: None,
        max_frames: Some(state.params.max_frames),
        min_sigma: Some(state.params.min_sigma),
        max_sigma: Some(state.params.max_sigma),
        top_percentage: Some(state.params.top_percentage),
        drizzle_scale: state.params.drizzle_scale,
        initial_rotation: 0.0,
        flat_inputs: p2s!(state.params.flat),
        dark_inputs: p2s!(state.params.dark),
        darkflat_inputs: p2s!(state.params.darkflat),
        bias_inputs: p2s!(state.params.bias),
        hot_pixel_map: p2s!(state.params.hot_pixel_map),
    }
}

macro_rules! check_cancel_status {
    () => {
        if is_cancel_requested!() {
            set_task_cancelled!();
            set_task_completed!();
            reset_cancel_status!();
            warn!("Task cancellation request detected. Stopping progress");
            panic!("Cancelling!");
        }
    };
}

async fn run_async() -> Result<()> {
    info!("Async task started");

    let output_filename = assemble_output_filename()?;
    let params = build_solhat_parameters();

    set_task_status!("Processing Master Flat", 2, 1);
    let master_flat = if let Some(inputs) = &params.flat_inputs {
        info!("Processing master flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status!();

    set_task_status!("Processing Master Dark Flat", 2, 1);
    let master_darkflat = if let Some(inputs) = &params.darkflat_inputs {
        info!("Processing master dark flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status!();

    set_task_status!("Processing Master Dark", 2, 1);
    let master_dark = if let Some(inputs) = &params.dark_inputs {
        info!("Processing master dark...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status!();

    set_task_status!("Processing Master Bias", 2, 1);
    let master_bias = if let Some(inputs) = &params.bias_inputs {
        info!("Processing master bias...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status!();

    info!("Creating process context struct");
    let mut context = ProcessContext::create_with_calibration_frames(
        &params,
        master_flat,
        master_darkflat,
        master_dark,
        master_bias,
    )?;

    check_cancel_status!();
    set_task_status!(
        "Computing Center-of-Mass Offsets",
        context.frame_records.len(),
        0
    );
    context.frame_records = frame_offset_analysis(&context, |_fr| {
        increment_status!();
        info!("frame_offset_analysis(): Frame processed.");
        check_cancel_status!();
    })?;

    set_task_status!("Frame Sigma Analysis", context.frame_records.len(), 0);
    context.frame_records = frame_sigma_analysis(&context, |fr| {
        increment_status!();
        info!(
            "frame_sigma_analysis(): Frame processed with sigma {}",
            fr.sigma
        );
        check_cancel_status!();
    })?;

    check_cancel_status!();
    set_task_status!("Applying Frame Limits", context.frame_records.len(), 0);
    context.frame_records = frame_limit_determinate(&context, |_fr| {
        info!("frame_limit_determinate(): Frame processed.");
        check_cancel_status!();
    })?;

    check_cancel_status!();
    set_task_status!(
        "Computing Parallactic Angle Rotations",
        context.frame_records.len(),
        0
    );
    context.frame_records = frame_rotation_analysis(&context, |fr| {
        increment_status!();
        info!(
            "Rotation for frame is {} degrees",
            fr.computed_rotation.to_degrees()
        );
        check_cancel_status!();
    })?;

    if context.frame_records.is_empty() {
        println!("Zero frames to stack. Cannot continue");
    } else {
        check_cancel_status!();
        set_task_status!("Stacking", context.frame_records.len(), 0);
        let drizzle_output = process_frame_stacking(&context, |_fr| {
            info!("process_frame_stacking(): Frame processed.");
            increment_status!();
        })?;

        check_cancel_status!();
        set_task_status!("Finalizing", 2, 1);
        let mut stacked_buffer = drizzle_output.get_finalized().unwrap();

        // Let the user know some stuff...
        let (stackmin, stackmax) = stacked_buffer.get_min_max_all_channel();
        info!(
            "    Stack Min/Max : {}, {} ({} images)",
            stackmin,
            stackmax,
            context.frame_records.len()
        );
        stacked_buffer.normalize_to_16bit();
        info!(
            "Final image size: {}, {}",
            stacked_buffer.width, stacked_buffer.height
        );

        // Save finalized image to disk
        set_task_status!("Saving", 2, 1);
        stacked_buffer.save(output_filename.to_string_lossy().as_ref())?;

        // The user will likely never see this actually appear on screen
        set_task_status!("Done", 1, 1);
    }

    set_task_completed!();

    Ok(())
}
