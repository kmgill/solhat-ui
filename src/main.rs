use anyhow::{anyhow, Result};
use gtk::gdk::Display;
use gtk::{gio, prelude::*, CssProvider, Label, STYLE_PROVIDER_PRIORITY_APPLICATION};
use gtk::{glib, Application, ApplicationWindow, Builder, Button};
use serde::{Deserialize, Serialize};
use solhat::drizzle::Scale;
use solhat::target::Target;
use std::borrow::Borrow;
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;

#[macro_use]
extern crate stump;

#[macro_use]
extern crate lazy_static;

/// Describes the parameters needed to run the SolHat algorithm
#[derive(Deserialize, Serialize, Default, Clone)]
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

lazy_static! {
    // Oh, this is such a hacky way to do it I hate it so much.
    // TODO: Learn the correct way to do this.
    static ref STATE: Arc<Mutex<ApplicationState>> = Arc::new(Mutex::new(ApplicationState::default()));
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
    Ok(application.run())
}

macro_rules! bind_open_clear {
    ($builder:expr, $window:expr, $open_id:expr, $clear_id:expr, $label_id:expr,$state_prop:ident, $opener:ident) => {{
        let btn_open: Button = if let Some(btn) = $builder.object($open_id) {
            btn
        } else {
            panic!("Failed to bind open button with id '{}'", $open_id);
        };

        let btn_clear: Button = if let Some(btn) = $builder.object($clear_id) {
            btn
        } else {
            panic!("Failed to bind clear button with id '{}'", $clear_id);
        };

        let label: Label = if let Some(lbl) = $builder.object($label_id) {
            lbl
        } else {
            panic!("Failed to bind label with id '{}'", $label_id);
        };

        let win = &$window;

        btn_open.connect_clicked(glib::clone!(@strong label, @weak win => move |_| {
            println!("Opening file");
            $opener("Open Ser File", &win,glib::clone!( @weak label => move|f| {
                println!("Opened: {:?}", f);
                label.set_label(f.file_name().unwrap().to_str().unwrap());
                STATE.lock().unwrap().params.$state_prop = Some(f);
            }));
        }));

        btn_clear.connect_clicked(glib::clone!(@strong label => move |_| {
            label.set_label("");
            let mut s = STATE.lock().unwrap();
            println!("Was: {:?}", s.params.$state_prop);
            s.params.$state_prop = None;
        }));
    }};
}

fn build_ui(application: &Application) {
    let ui_src = include_str!("../assets/solhat.ui");
    let builder = Builder::from_string(ui_src);

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

    let btn_output_open: Button = if let Some(btn) = builder.object("btn_output_folder_open") {
        btn
    } else {
        panic!("Failed to bind output folderopen button with id 'btn_output_folder_open'");
    };

    let lbl_output_folder: Label = if let Some(lbl) = builder.object("lbl_output_folder") {
        lbl
    } else {
        panic!("Failed to bind label with id 'lbl_output_folder'");
    };

    let lbl_output_filename: Label = if let Some(lbl) = builder.object("lbl_output_filename") {
        lbl
    } else {
        panic!("Failed to bind label with id 'lbl_output_filename'");
    };

    btn_output_open.connect_clicked(
        glib::clone!(@strong lbl_output_folder, @weak window => move |_| {
            println!("Opening file");
            open_folder("Open Ser File", &window,glib::clone!( @weak lbl_output_folder, @weak lbl_output_filename => move|f| {
                println!("Opened: {:?}", f);
                lbl_output_folder.set_label(f.to_str().unwrap());
                STATE.lock().unwrap().params.output_dir = Some(f);
                lbl_output_filename.set_label(assemble_output_filename().unwrap().to_str().unwrap());
                //
            }));
        }),
    );

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
        output_dir
    } else {
        return Err(anyhow!("Output directory not set"));
    };

    let base_filename = if let Some(input_file) = &state.params.light {
        Path::new(input_file.file_name().unwrap())
            .file_stem()
            .unwrap()
    } else {
        return Err(anyhow!("Input light file not provided"));
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
    let output_path: PathBuf = Path::new(output_dir).join(output_filename);
    Ok(output_path)
}
