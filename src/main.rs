use anyhow::{anyhow, Result};
use gtk::ffi::gtk_picture_new_for_filename;
use gtk::gdk::{Display, Texture};
use gtk::gdk_pixbuf::{Colorspace, Pixbuf};
use gtk::{
    gio, prelude::*, Adjustment, ComboBoxText, CssProvider, Entry, Label, Picture, ProgressBar,
    SpinButton, Spinner, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use gtk::{glib, Application, ApplicationWindow, Builder, Button};
use image::Progress;
use itertools::iproduct;
use serde::{Deserialize, Serialize};
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
use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

#[macro_use]
extern crate stump;

#[macro_use]
extern crate lazy_static;

/// Describes the parameters needed to run the SolHat algorithm
#[derive(Deserialize, Serialize, Clone)]
struct ParametersState {
    light: Option<PathBuf>,
    dark: Option<PathBuf>,
    flat: Option<PathBuf>,
    darkflat: Option<PathBuf>,
    bias: Option<PathBuf>,
    hot_pixel_map: Option<PathBuf>,
    output_dir: Option<PathBuf>,
    freetext: String,
    obs_latitude: f64,
    obs_longitude: f64,
    target: Target,
    obj_detection_threshold: f64,
    drizzle_scale: Scale,
    max_frames: usize,
    min_sigma: f64,
    max_sigma: f64,
    top_percentage: f64,
}

impl Default for ParametersState {
    fn default() -> Self {
        Self {
            light: Default::default(),
            dark: Default::default(),
            flat: Default::default(),
            darkflat: Default::default(),
            bias: Default::default(),
            hot_pixel_map: Default::default(),
            output_dir: Default::default(),
            freetext: Default::default(),
            obs_latitude: 34.0,
            obs_longitude: -118.0,
            target: Target::Sun,
            obj_detection_threshold: 2000.0,
            drizzle_scale: Scale::Scale1_0,
            max_frames: 5000,
            min_sigma: 0.0,
            max_sigma: 2000.0,
            top_percentage: 10.0,
        }
    }
}

/// Describes the state of the UI
#[derive(Deserialize, Serialize, Default, Clone)]
struct UiState {
    last_opened_folder: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, Default, Clone)]
struct ApplicationState {
    pub params: ParametersState,
    pub ui: UiState,
}

impl ApplicationState {
    pub fn load_from_userhome() -> Result<Self> {
        let config_file_path = dirs::home_dir().unwrap().join(".solhat/shconfig.toml");
        if config_file_path.exists() {
            info!(
                "Window state config file exists at path: {:?}",
                config_file_path
            );
            let t = std::fs::read_to_string(config_file_path)?;
            Ok(toml::from_str(&t)?)
        } else {
            warn!("Window state config file does not exist. Will be created on exit");
            Err(anyhow!("Config file does not exist"))
        }
    }

    pub fn save_to_userhome(&self) -> Result<()> {
        let toml_str = toml::to_string(&self).unwrap();
        let solhat_config_dir = dirs::home_dir().unwrap().join(".solhat/");
        if !solhat_config_dir.exists() {
            fs::create_dir(&solhat_config_dir)?;
        }
        let config_file_path = solhat_config_dir.join("shconfig.toml");
        let mut f = File::create(config_file_path)?;
        f.write_all(toml_str.as_bytes())?;
        debug!("{}", toml_str);
        Ok(())
    }
}

enum TaskStatus {
    TaskPercentage(String, usize, usize),
}

#[derive(Default)]
struct TaskStatusContainer {
    status: Option<TaskStatus>,
}

lazy_static! {
    // Oh, this is such a hacky way to do it I hate it so much.
    // TODO: Learn the correct way to do this.
    static ref STATE: Arc<Mutex<ApplicationState>> = Arc::new(Mutex::new(ApplicationState::default()));

    static ref TASK_STATUS_QUEUE: Arc<Mutex<TaskStatusContainer>> =
        Arc::new(Mutex::new(TaskStatusContainer::default()));
}

macro_rules! get_state_param {
    ($prop:ident) => {
        STATE.lock().unwrap().params.$prop
    };
}

macro_rules! set_state_param {
    ($prop:ident, $value:expr) => {
        STATE.lock().unwrap().params.$prop = $value;
    };
}

macro_rules! set_state_ui {
    ($prop:ident, $value:expr) => {
        STATE.lock().unwrap().ui.$prop = $value;
    };
}

macro_rules! clear_last_opened_folder {
    () => {
        set_state_ui!(last_opened_folder, None);
    };
}

macro_rules! set_last_opened_folder {
    ($dir:expr) => {
        set_state_ui!(last_opened_folder, Some($dir));
        info!("Setting last opened folder to {:?}", $dir);
    };
}

#[tokio::main]
async fn main() -> Result<glib::ExitCode> {
    stump::set_min_log_level(stump::LogEntryLevel::DEBUG);
    info!("Starting SolHat-UI");

    let application = gtk::Application::new(Some("com.apoapsys.solhat"), Default::default());

    application.connect_startup(|app| {
        // The CSS "magic" happens here.
        let provider = CssProvider::new();
        provider.load_from_data(include_str!("../assets/styles.css"));
        // We give the CssProvided to the default screen so the CSS rules we added
        // can be applied to our window.
        gtk::style_context_add_provider_for_display(
            &Display::default().expect("Could not connect to a display."),
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

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
    ($builder:expr,$ser_file_path:expr) => {
        let pic: Picture = bind_object!($builder, "img_preview");

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
    ($builder:expr, $window:expr, $open_id:expr, $clear_id:expr, $label_id:expr,$state_prop:ident, $opener:ident) => {{
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
                update_preview_from_ser_file!(builder, f);
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
        spn_obj.set_value(get_state_param!($state_prop) as f64);
        spn_obj.connect_changed(|e| {
            set_state_param!($state_prop, e.value() as $type);
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

    bind_open_clear!(
        builder,
        window,
        "btn_light_open",
        "btn_light_clear",
        "lbl_light",
        light,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_dark_open",
        "btn_dark_clear",
        "lbl_dark",
        dark,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_flat_open",
        "btn_flat_clear",
        "lbl_flat",
        flat,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_darkflat_open",
        "btn_darkflat_clear",
        "lbl_darkflat",
        darkflat,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_bias_open",
        "btn_bias_clear",
        "lbl_bias",
        flat,
        open_ser_file
    );

    bind_open_clear!(
        builder,
        window,
        "btn_hotpixelmap_open",
        "btn_hotpixelmap_clear",
        "lbl_hotpixelmap",
        hot_pixel_map,
        open_ser_file
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
    let update_state_callback = glib::clone!(@weak label, @weak start => @default-return Continue(true), move || {
        let proc_status = TASK_STATUS_QUEUE.lock().unwrap();
        match &proc_status.status {
            Some(TaskStatus::TaskPercentage(task_name, len, cnt)) => {
                let pct = if *len > 0 {
                    *cnt as f64 / *len as f64
                } else {
                    0.0
                };
                label.set_visible(true);
                progress.set_visible(true);
                label.set_label(&task_name);
                progress.set_fraction(pct);
                start.set_sensitive(false);
            },
            None => {
                label.set_visible(false);
                progress.set_visible(false);
                start.set_sensitive(true);
            }
        };
        // let spn: Spinner = bind_object!(b, "prg_task_progress");

        Continue(true)
    });

    let source_id = glib::timeout_add_local(Duration::from_millis(250), update_state_callback);

    update_output_filename!(builder);

    // If there's a file in the parameters state already (such as from saved state),
    // we need to update the preview pane.
    if let Some(light_path) = &STATE.lock().unwrap().params.light {
        update_preview_from_ser_file!(builder, light_path);
    }

    window.present();
}

fn open_ser_file<F>(title: &str, window: &ApplicationWindow, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    let ser_filter = gtk::FileFilter::new();
    ser_filter.add_mime_type("video/ser");
    ser_filter.set_name(Some("SER"));
    // Add filter

    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Open")
        .modal(true)
        // .default_filter(&ser_filter)
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

fn increment_status() {
    let mut stat = TASK_STATUS_QUEUE.lock().unwrap();
    match &mut stat.status {
        Some(TaskStatus::TaskPercentage(name, len, val)) => {
            info!("Updating task status with value {}", val);
            stat.status = Some(TaskStatus::TaskPercentage(name.to_owned(), *len, *val + 1))
        }
        None => {}
    }
}

fn set_task_status(task_name: &str, len: usize, cnt: usize) {
    TASK_STATUS_QUEUE.lock().unwrap().status =
        Some(TaskStatus::TaskPercentage(task_name.to_owned(), len, cnt))
}

fn set_task_completed() {
    TASK_STATUS_QUEUE.lock().unwrap().status = None
}

async fn run_async() -> Result<()> {
    info!("Async task started");

    let output_filename = assemble_output_filename()?;
    let params = build_solhat_parameters();

    set_task_status("Processing Master Flat", 2, 1);
    let master_flat = if let Some(inputs) = &params.flat_inputs {
        info!("Processing master flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    set_task_status("Processing Master Dark Flat", 2, 1);
    let master_darkflat = if let Some(inputs) = &params.darkflat_inputs {
        info!("Processing master dark flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    set_task_status("Processing Master Dark", 2, 1);
    let master_dark = if let Some(inputs) = &params.dark_inputs {
        info!("Processing master dark...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    // set_task_status("Processing Master Bias", 2, 1);
    let master_bias = if let Some(inputs) = &params.bias_inputs {
        info!("Processing master bias...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    info!("Creating process context struct");
    let mut context = ProcessContext::create_with_calibration_frames(
        &params,
        master_flat,
        master_darkflat,
        master_dark,
        master_bias,
    )?;

    set_task_status("Frame Sigma Analysis", context.frame_records.len(), 0);
    context.frame_records = frame_sigma_analysis(&context, |_fr| {
        increment_status();
        info!("frame_sigma_analysis(): Frame processed.")
    })?;

    set_task_status("Applying Frame Limits", context.frame_records.len(), 0);
    context.frame_records = frame_limit_determinate(&context, |_fr| {
        info!("frame_limit_determinate(): Frame processed.")
    })?;

    set_task_status(
        "Computing Parallactic Angle Rotations",
        context.frame_records.len(),
        0,
    );
    context.frame_records = frame_rotation_analysis(&context, |fr| {
        increment_status();
        info!(
            "Rotation for frame is {} degrees",
            fr.computed_rotation.to_degrees()
        );
    })?;

    set_task_status(
        "Computing Center-of-Mass Offsets",
        context.frame_records.len(),
        0,
    );
    context.frame_records = frame_offset_analysis(&context, |_fr| {
        increment_status();
        info!("frame_offset_analysis(): Frame processed.")
    })?;

    if context.frame_records.is_empty() {
        println!("Zero frames to stack. Cannot continue");
    } else {
        set_task_status("Stacking", context.frame_records.len(), 0);
        let drizzle_output = process_frame_stacking(&context, |_fr| {
            info!("process_frame_stacking(): Frame processed.");
            increment_status();
        })?;

        set_task_status("Finalizing", 2, 1);
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
        set_task_status("Saving", 2, 1);
        stacked_buffer.save(output_filename.to_string_lossy().as_ref())?;

        // The user will likely never see this actually appear on screen
        set_task_status("Done", 1, 1);
    }

    set_task_completed();

    Ok(())
}
