#[macro_use]
mod state;
use state::*;

mod cancel;
use cancel::*;

mod taskstatus;
use taskstatus::*;

use anyhow::Result;
use gtk::gdk::Display;
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::glib::{MainContext, Priority, Sender, Type};
#[allow(deprecated)]
use gtk::{
    gio, prelude::*, Adjustment, ComboBoxText, CssProvider, Entry, Label, Picture, ProgressBar,
    ScrolledWindow, SpinButton, TextBuffer, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use gtk::{glib, AlertDialog, Application, ApplicationWindow, Builder, Button, CheckButton};
use itertools::iproduct;
use sciimg::prelude::*;
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
use solhat::threshtest::compute_rgb_threshtest_image;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

#[macro_use]
extern crate stump;

#[macro_use]
extern crate lazy_static;

#[tokio::main]
async fn main() -> Result<glib::ExitCode> {
    stump::set_min_log_level(stump::LogEntryLevel::DEBUG);
    info!("Starting SolHat-UI");

    let application = gtk::Application::new(Some("com.apoapsys.solhat"), Default::default());

    application.connect_activate(build_styles);
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

fn build_styles(_application: &Application) {
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

    let (process_sender, process_receiver) = MainContext::channel(Priority::default());

    let window: ApplicationWindow = builder
        .object("SolHatApplicationMain")
        .expect("Couldn't get window");
    window.set_application(Some(application));

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

    ////////
    // Decorrelated Colors
    ////////
    let chk_decorr_colors: CheckButton = bind_object!(builder, "chk_decorrelated_color");
    chk_decorr_colors.connect_toggled(|e: &CheckButton| {
        set_state_param!(decorrelated_colors, e.is_active());
        info!("Decorrelated Colors: {}", e.is_active())
    });

    ////////
    // Threshold Test
    ////////
    let btn_thresh_test: Button = bind_object!(builder, "btn_thresh_test");
    let (stat_sender, stat_receiver) = MainContext::channel(Priority::default());
    let (pix_sender, pix_receiver) = MainContext::channel(Priority::default());
    let ps = process_sender.clone();
    btn_thresh_test.connect_clicked(glib::clone!(@weak window, @weak b as builder => move |_| {
        info!("Thresh test clicked");
        let stat_sender = stat_sender.clone();
        let pix_sender = pix_sender.clone();
        let ps =  ps.clone();

        thread::spawn(move || {
            stat_sender.send(false).expect("Could not send through channel");
            if let Ok(buffer) = run_thresh_test(ps) {
                pix_sender.send(Some(buffer)).expect("Failed to send pixbuf through channel");
            } else {
                pix_sender.send(None).expect("Failed to send pixbuf through channel");
            }
            stat_sender.send(true).expect("Could not send through channel");
        });

    }));
    stat_receiver.attach(
        None,
        glib::clone!(@weak btn_thresh_test => @default-return Continue(false),
                    move |enable_button| {
                        btn_thresh_test.set_sensitive(enable_button);
                        Continue(true)
                    }
        ),
    );
    pix_receiver.attach(
        None,
        glib::clone!(@weak window, @weak b as builder => @default-return Continue(false),
                    move |buffer_opt| {
                        if let Some(buffer) = buffer_opt {
                            let pix = image_to_picture(&buffer).expect("Failed to convert imagebuffer to pixbuf");
                            let pic: Picture = bind_object!(builder, "img_preview_light");
                            pic.set_pixbuf(Some(&pix));
                        } else {
                            let info_dialog = AlertDialog::builder()
                                                            .modal(true)
                                                            .message("Error")
                                                            .detail("No light input specified. Please do so before continuing")
                                                            .build();

                            info_dialog.show(Some(&window));
                        }
                        Continue(true)
                    }
        ),
    );

    ////////
    // Task Monitor
    ////////
    // let b = Rc::new(Cell::new(builder.clone()));
    let label: Label = bind_object!(b, "lbl_task_status");
    let progress: ProgressBar = bind_object!(b, "prg_task_progress");
    let cancel: Button = bind_object!(b, "btn_cancel");

    cancel.connect_clicked(move |_| {
        set_request_cancel();
    });

    ////////
    // Process Execution
    ////////

    update_execute_state!(builder);
    let start: Button = bind_object!(builder, "btn_execute");
    let ps = process_sender.clone();
    start.connect_clicked(move |_| {
        debug!("Start has been clicked");
        let ps = ps.clone();
        tokio::spawn(async move {
            {
                ps.send(TaskStatusContainer {
                    status: Some(TaskStatus::TaskPercentage("Starting".to_owned(), 0, 0)),
                })
                .expect("Failed to sent task status");
                run_async(ps).await.unwrap(); //.await.unwrap();
            }
        });
    });
    process_receiver.attach(
        None,
        glib::clone!(@weak label, @weak start, @weak btn_thresh_test => @default-return Continue(false),
            move |proc_status| {
                match &proc_status.status {
                    Some(TaskStatus::TaskPercentage(task_name, len, cnt)) => {
                        let pct = if *len > 0 {
                            *cnt as f64 / *len as f64
                        } else {
                            0.0
                        };
                        // label.set_visible(true);
                        progress.set_visible(true);
                        cancel.set_visible(true);
                        label.set_label(&task_name);
                        progress.set_fraction(pct);
                        start.set_sensitive(false);
                        cancel.set_sensitive(true);
                        btn_thresh_test.set_sensitive(false);
                    },
                    None => {
                        // label.set_visible(false);
                        progress.set_visible(false);
                        cancel.set_visible(false);
                        start.set_sensitive(true);
                        cancel.set_sensitive(false);
                        btn_thresh_test.set_sensitive(true);
                        label.set_label("Ready");
                    }
                };


                Continue(true)
            }
        ),
    );

    ////////
    // Logging
    ////////
    //
    let log_buffer: TextBuffer = bind_object!(builder, "txt_log_buffer");
    let scrl_log_output: ScrolledWindow = bind_object!(builder, "scrl_log_output");
    let (log_sender, log_receiver) = MainContext::channel(Priority::default());

    stump::set_print(move |s| {
        println!("{}", s);
        log_sender.send(s.to_owned()).expect("Failed to send log");
    });

    log_receiver.attach(
        None,
        glib::clone!(@weak log_buffer, @weak scrl_log_output => @default-return Continue(false),
            move |log_entry| {

                let mut end = log_buffer.end_iter();
                log_buffer.insert(&mut end, "\n");
                log_buffer.insert(&mut end, &log_entry);

                // Scroll to bottom
                let vadjustment = scrl_log_output.vadjustment();
                vadjustment.set_value(vadjustment.upper());

                Continue(true)
            }
        ),
    );

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
    image_to_picture(&ser_frame.buffer)
}

fn image_to_picture(image: &Image) -> Result<Pixbuf> {
    let mut copied = image.clone();
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

#[allow(dead_code)]
fn imagebuffer_to_picture(buffer: &ImageBuffer) -> Result<Pixbuf> {
    let mut copied = buffer.clone();
    copied.normalize_mut(0.0, 255.0);

    let pix = Pixbuf::new(
        Colorspace::Rgb,
        false,
        8,
        copied.width as i32,
        copied.height as i32,
    )
    .unwrap();

    iproduct!(0..copied.height, 0..copied.width).for_each(|(y, x)| {
        let v = copied.get(x, y);
        pix.put_pixel(x as u32, y as u32, v as u8, v as u8, v as u8, 255);
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

fn build_solhat_parameters() -> Result<ProcessParameters> {
    let state = STATE.lock().unwrap();

    if state.params.light.is_none() {
        return Err(anyhow!("No light input identified"));
    }
    Ok(ProcessParameters {
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
    })
}

fn build_solhat_context(sender: &Sender<TaskStatusContainer>) -> Result<ProcessContext> {
    let params = build_solhat_parameters()?;

    set_task_status(&sender, "Processing Master Flat", 2, 1);
    let master_flat = if let Some(inputs) = &params.flat_inputs {
        info!("Processing master flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(&sender);

    set_task_status(&sender, "Processing Master Dark Flat", 2, 1);
    let master_darkflat = if let Some(inputs) = &params.darkflat_inputs {
        info!("Processing master dark flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(&sender);

    set_task_status(&sender, "Processing Master Dark", 2, 1);
    let master_dark = if let Some(inputs) = &params.dark_inputs {
        info!("Processing master dark...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(&sender);

    set_task_status(&sender, "Processing Master Bias", 2, 1);
    let master_bias = if let Some(inputs) = &params.bias_inputs {
        info!("Processing master bias...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(&sender);

    info!("Creating process context struct");
    let context = ProcessContext::create_with_calibration_frames(
        &params,
        master_flat,
        master_darkflat,
        master_dark,
        master_bias,
    )?;

    Ok(context)
}

pub fn check_cancel_status(sender: &Sender<TaskStatusContainer>) {
    if is_cancel_requested() {
        set_task_cancelled();
        set_task_completed(sender);
        reset_cancel_status();
        warn!("Task cancellation request detected. Stopping progress");
        panic!("Cancelling!");
    }
}

lazy_static! {
    // NOTE: Concurrent processing threads will stomp on each other, but at least
    // they'll do it in proper turn.  Also, this is stupid and can't stay this way.
    static ref COUNTER: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
}

async fn run_async(master_sender: Sender<TaskStatusContainer>) -> Result<()> {
    info!("Async task started");

    let output_filename = assemble_output_filename()?;
    // let params = build_solhat_parameters();
    let mut context = build_solhat_context(&master_sender)?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    check_cancel_status(&master_sender);
    let frame_count = context.frame_records.len();
    let sender = master_sender.clone();
    set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, 0);
    context.frame_records = frame_offset_analysis(&context, move |_fr| {
        info!("frame_offset_analysis(): Frame processed.");
        check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c = *c + 1;
        set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, *c)
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////
    check_cancel_status(&master_sender);
    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    set_task_status(&sender, "Frame Sigma Analysis", frame_count, 0);
    context.frame_records = frame_sigma_analysis(&context, move |fr| {
        info!(
            "frame_sigma_analysis(): Frame processed with sigma {}",
            fr.sigma
        );
        check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c = *c + 1;
        set_task_status(&sender, "Frame Sigma Analysis", frame_count, *c)
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    check_cancel_status(&master_sender);
    set_task_status(&sender, "Applying Frame Limits", frame_count, 0);
    context.frame_records = frame_limit_determinate(&context, move |_fr| {
        info!("frame_limit_determinate(): Frame processed.");
        check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c = *c + 1;
        set_task_status(&sender, "Applying Frame Limits", frame_count, *c)
    })?;

    /////////////////////////////////////////////////////////////
    /////////////////////////////////////////////////////////////

    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    check_cancel_status(&master_sender);
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
        check_cancel_status(&sender);

        let mut c = COUNTER.lock().unwrap();
        *c = *c + 1;
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
        check_cancel_status(&master_sender);
        set_task_status(&sender, "Stacking", frame_count, 0);
        let drizzle_output = process_frame_stacking(&context, move |_fr| {
            info!("process_frame_stacking(): Frame processed.");
            check_cancel_status(&sender);

            let mut c = COUNTER.lock().unwrap();
            *c = *c + 1;
            set_task_status(&sender, "Stacking", frame_count, *c)
        })?;

        check_cancel_status(&master_sender);
        set_task_status(&master_sender, "Finalizing", 2, 1);
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
        set_task_status(&master_sender, "Saving", 2, 1);
        stacked_buffer.save(output_filename.to_string_lossy().as_ref())?;

        // The user will likely never see this actually appear on screen
        set_task_status(&master_sender, "Done", 1, 1);
    }

    set_task_completed(&master_sender);

    Ok(())
}

fn run_thresh_test(master_sender: Sender<TaskStatusContainer>) -> Result<Image> {
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
