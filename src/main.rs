use anyhow::{anyhow, Result};
use glib::Downgrade;
use gtk::gdk::Display;
use gtk::{gio, prelude::*, CssProvider, Label, STYLE_PROVIDER_PRIORITY_APPLICATION};
use gtk::{glib, Application, ApplicationWindow, Builder, Button};
use serde::{Deserialize, Serialize};
use solhat::drizzle::Scale;
use solhat::target::Target;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

#[derive(Deserialize, Serialize, Default, Clone)]
struct OptionsState {
    light: Option<PathBuf>,
    dark: Option<String>,
    flat: Option<String>,
    darkflat: Option<String>,
    bias: Option<String>,
    hot_pixel_map: Option<String>,
    output_dir: Option<String>,
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

fn main() -> glib::ExitCode {
    let application = gtk::Application::new(Some("com.apoapsys.solhat"), Default::default());
    //
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
    application.run()
}

type SharedState = Rc<Cell<OptionsState>>;

fn build_ui(application: &Application) {
    let mut state = Rc::new(Cell::new(OptionsState::default()));

    let ui_src = include_str!("../assets/solhat.ui");
    let builder = Builder::from_string(ui_src);

    let window: ApplicationWindow = builder
        .object("SolHatApplicationMain")
        .expect("Couldn't get window");
    window.set_application(Some(application));

    build_inputs_ui(&builder, &window, &mut state).expect("Failed to create inputs UI");

    window.present();
}

fn open_ser_file<F>(title: &str, window: &ApplicationWindow, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    let ser_filter = gtk::FileFilter::new();
    ser_filter.add_mime_type("video/*");
    ser_filter.set_name(Some("Video"));
    // Add filter

    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Open")
        .modal(true)
        .build();

    dialog.open(Some(window), gio::Cancellable::NONE, move |file| {
        if let Ok(file) = file {
            let filename = file.path().expect("Couldn't get file path");

            callback(filename);
        }
    });
}

fn build_inputs_ui(
    builder: &Builder,
    window: &ApplicationWindow,
    state: &mut SharedState,
) -> Result<()> {
    let btn_light_open: Button = builder
        .object("btn_light_open")
        .expect("Could not bind btn_light_open");

    let btn_light_clear: Button = builder
        .object("btn_light_clear")
        .expect("Could not bind btn_light_clear");

    let lbl_light: Label = builder
        .object("lbl_light")
        .expect("Could not bind lbl_light");

    btn_light_open.connect_clicked(
        glib::clone!(@strong lbl_light, @strong state, @weak window => move |_| {
            open_ser_file("Open Ser File", &window,glib::clone!( @weak lbl_light => move|f| {
                println!("Opened: {:?}", f);
                lbl_light.set_label(f.file_name().unwrap().to_str().unwrap());

            }));

        }),
    );

    btn_light_clear.connect_clicked(
        glib::clone!(@strong lbl_light,@strong state, @weak window => move |_| {
            lbl_light.set_label("");
        }),
    );

    Ok(())
}
