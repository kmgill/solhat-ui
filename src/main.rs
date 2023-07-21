#[macro_use]
mod state;
use gtk::gdk_pixbuf::PixbufLoader;
use state::*;

mod cancel;
use cancel::*;

mod taskstatus;
use taskstatus::*;

mod analysis;
use analysis::*;

mod process;

mod conversion;
use conversion::*;

use anyhow::Result;
use gtk::gdk::Display;
use gtk::glib::{MainContext, Priority, Type};
#[allow(deprecated)]
use gtk::{
    gio, prelude::*, Adjustment, ComboBoxText, CssProvider, Entry, Label, Picture, ProgressBar,
    ScrolledWindow, SpinButton, TextBuffer, STYLE_PROVIDER_PRIORITY_APPLICATION,
};
use gtk::{glib, AlertDialog, Application, ApplicationWindow, Builder, Button, CheckButton, Notebook};
use solhat::drizzle::Scale;
use solhat::target::Target;
use std::ffi::OsStr;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::thread;
use solhat::ser::SerFile;

#[macro_use]
extern crate stump;

#[macro_use]
extern crate lazy_static;


const TAB_ID_LIGHT:i32 = 0;
const TAB_ID_DARK:i32 = 1;
const TAB_ID_FLAT:i32 = 2;
const TAB_ID_FLATDARK:i32 = 3;
const TAB_ID_BIAS:i32 = 4;
const TAB_ID_ANALYSIS:i32 = 5;

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
        

        if let Some(ext) = $ser_file_path.extension() {
            match ext.to_str().unwrap() {
                "ser" => {

                    let (pix_sender, pix_receiver) = MainContext::channel(Priority::default());

                    thread::spawn(move || {
                        // Load the ser file, grab the first frame, then send it over to the message loop
                        let ser_file = SerFile::load_ser($ser_file_path.to_str().unwrap()).unwrap();
                        let first_image = ser_file.get_frame(0).unwrap();
                        pix_sender.send(Some(first_image)).expect("Failed to send pixbuf through channel");
                    });

                    let b = $builder.clone();
                    pix_receiver.attach(
                        None,
                        glib::clone!( @weak b as builder => @default-return Continue(false),
                                    move |pix_opt| {

                                        if let Some(ser_frame) = pix_opt {
                                            // If it's a valid image (and not None), convert it to
                                            // gtk::Picture and display it
                                            let pix = ser_frame_to_picture(&ser_frame).unwrap();
                                            let pic: Picture = bind_object!(builder, $preview_id);
                                            pic.set_pixbuf(Some(&pix));
                                        }else {
                                            error!("Failed to load preview image");
                                        }
                                        Continue(true)
                                    }
                                ),
                            );
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
    ($builder:expr, $window:expr, $open_id:expr, $clear_id:expr, $label_id:expr, $preview_id:expr, $state_prop:ident, $opener:ident, $tab_id:expr) => {{
        let btn_open: Button = bind_object!($builder, $open_id);
        let btn_clear: Button = bind_object!($builder, $clear_id);
        let label: Label = bind_object!($builder, $label_id);

        if let Some(prop) = get_state_param!($state_prop) {
            label.set_label(prop.file_name().unwrap().clone().to_str().unwrap());
        }

        let win = &$window;

        let b = $builder.clone();
        btn_open.connect_clicked(glib::clone!(@strong label, @weak win, @weak b as builder => move |_| {
            debug!("Opening file");
            
            let p = get_state_param!($state_prop);
            let last_opened = if let Some(p) = p {
                Some(p)
            } else {
                get_last_opened_folder!()
            };

            info!("Last opened folder: {:?}", last_opened);

            $opener("Open Ser File", &win,last_opened, glib::clone!( @weak label => move|f| {
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

                // Set tab_id if no tab is needed (such as for non image files)
                if $tab_id >= 0 {
                    let notebook : Notebook = bind_object!(builder, "notebook_previews");
                    notebook.set_page($tab_id);
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

    if let Ok(mut ss) = ApplicationState::load_from_userhome() {
        ss.validate_paths();
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
        open_ser_file,
        TAB_ID_LIGHT
    );

    bind_open_clear!(
        builder,
        window,
        "btn_dark_open",
        "btn_dark_clear",
        "lbl_dark",
        "img_preview_dark",
        dark,
        open_ser_file,
        TAB_ID_DARK
    );

    bind_open_clear!(
        builder,
        window,
        "btn_flat_open",
        "btn_flat_clear",
        "lbl_flat",
        "img_preview_flat",
        flat,
        open_ser_file,
        TAB_ID_FLAT
    );

    bind_open_clear!(
        builder,
        window,
        "btn_darkflat_open",
        "btn_darkflat_clear",
        "lbl_darkflat",
        "img_preview_darkflat",
        darkflat,
        open_ser_file,
        TAB_ID_FLATDARK
    );

    bind_open_clear!(
        builder,
        window,
        "btn_bias_open",
        "btn_bias_clear",
        "lbl_bias",
        "img_preview_bias",
        bias,
        open_ser_file,
        TAB_ID_BIAS
    );

    bind_open_clear!(
        builder,
        window,
        "btn_hotpixelmap_open",
        "btn_hotpixelmap_clear",
        "lbl_hotpixelmap",
        "",
        hot_pixel_map,
        open_toml_file,
        -1
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
            open_folder("Select Output Folder", &window, get_last_opened_folder!(), glib::clone!( @weak lbl_output_folder, @weak b as builder => move|f| {
                debug!("Opened: {:?}", f);
                lbl_output_folder.set_label(f.to_str().unwrap());
                set_state_param!(output_dir, Some(f.to_owned()));
                update_output_filename!(builder);
                set_last_opened_folder!(f);
            }));
        }),
    );

    ////////
    // Free text
    ////////
    #[allow(clippy::redundant_clone)]
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
        Target::None => combo_target.set_active_id(Some("2")),
    };
    combo_target.connect_changed(glib::clone!(@weak window, @weak b as builder => move |e| {
        set_state_param!(target, match e.active_id().unwrap().to_string().as_str() {
            "0" => Target::Sun,
            "1" => Target::Moon,
            "2" => Target::None,
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
    bind_spinner!(builder, "spn_window_size", analysis_window_size, usize);

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
            if let Ok(buffer) = threshold::run_thresh_test(ps) {
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
    // Analysis
    ////////
    let btn_analysis: Button = bind_object!(builder, "btn_analysis");
    let (ana_stat_sender, ana_stat_receiver) = MainContext::channel(Priority::default());
    let (ana_data_sender, ana_data_receiver) = MainContext::channel(Priority::default());
    let ps = process_sender.clone();
    btn_analysis.connect_clicked(glib::clone!(@weak window, @weak b as builder => move |_| {
        info!("Analysis clicked");
        let stat_sender = ana_stat_sender.clone();
        let data_sender = ana_data_sender.clone();
        let ps =  ps.clone();

        thread::spawn(move || {
            stat_sender.send(false).expect("Could not send through channel");
            match sigma::run_sigma_analysis(ps) {
                Ok(data_series) => data_sender.send(Some(data_series)).expect("Failed to send pixbuf through channel"),
                Err(TaskCompletion::Error(why)) => {
                    error!("Task error: {:?}", why);
                    data_sender.send(None).expect("Failed to send pixbuf through channel")
                },
                Err(_) => {} // Ignore
            };
            stat_sender.send(true).expect("Could not send through channel");
        });

    }));
    ana_stat_receiver.attach(
        None,
        glib::clone!(@weak btn_analysis => @default-return Continue(false),
                    move |enable_button| {
                        btn_analysis.set_sensitive(enable_button);
                        Continue(true)
                    }
        ),
    );

    ana_data_receiver.attach(
        None,
        glib::clone!(@weak window, @weak b as builder => @default-return Continue(false),
                    move |data_series| {
                        if let Some(data_series) = &data_series {
                            let pic: Picture = bind_object!(builder, "img_analysis");
                            let pic_label : Label =  bind_object!(builder, "lbl_analysis");
                            let notebook : Notebook = bind_object!(builder, "notebook_previews");
                            
                            // Try to find out the size dynamically. Currently, using
                            // pic.width()/pic.height() don't work before it's been set to something.
                            // Also, using notebook width/height seems sorta hackish.
                            let svg_string = sigma::create_chart(data_series,notebook.width() as isize,notebook.height() as isize).unwrap();
                            let loader = PixbufLoader::new();
                            loader.write(svg_string.as_bytes()).expect("Failed to write svg to pixbuf loader");
                            loader.close().expect("Failed to load svg");
                            let pixbuf = loader.pixbuf().unwrap();
                            pic.set_pixbuf(Some(&pixbuf));
                            pic.set_visible(true);
                            pic_label.set_visible(false);
                            notebook.set_page(TAB_ID_ANALYSIS);
                        } else {
                            let info_dialog = AlertDialog::builder()
                                                            .modal(true)
                                                            .message("Error")
                                                            .detail("Unable to perform sigma analysis")
                                                            .build();

                            info_dialog.show(Some(&window));
                        }
                        Continue(true)
                    }
        ),
    );

    // let pic: Picture = bind_object!(builder, "img_preview_light");
    // pic.connect_width_request_notify(|f| {
    //     info!("Width!");
    // });
    // pic.connect_scale_factor_notify(|f| {
    //     info!("Scale factor!");
    // });

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
    #[allow(clippy::redundant_clone)]
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
                process::run_async(ps, assemble_output_filename().unwrap()).await.unwrap(); //.await.unwrap();
            }
        });
    });
    process_receiver.attach(
        None,
        glib::clone!(@weak label, @weak start, @weak btn_thresh_test, @weak btn_analysis => @default-return Continue(false),
            move |proc_status| {
                match &proc_status.status {
                    Some(TaskStatus::TaskPercentage(task_name, len, cnt)) => {
                        if *len > 0 {
                            progress.set_fraction(*cnt as f64 / *len as f64);
                        } else {
                            progress.pulse();
                        };
                        // label.set_visible(true);
                        progress.set_visible(true);
                        cancel.set_visible(true);
                        label.set_label(task_name);
                        start.set_sensitive(false);
                        cancel.set_sensitive(true);
                        btn_thresh_test.set_sensitive(false);
                        btn_analysis.set_sensitive(false);
                    },
                    None => {
                        // label.set_visible(false);
                        progress.set_visible(false);
                        cancel.set_visible(false);
                        start.set_sensitive(true);
                        cancel.set_sensitive(false);
                        btn_thresh_test.set_sensitive(true);
                        btn_analysis.set_sensitive(true);
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
    
    if let Some(light_path) = get_state_param!(light) {
        update_preview_from_ser_file!(builder, light_path, "img_preview_light");
    }
    if let Some(dark_path) = get_state_param!(dark) {
        update_preview_from_ser_file!(builder, dark_path, "img_preview_dark");
    }
    if let Some(flat_path) = get_state_param!(flat) {
        update_preview_from_ser_file!(builder, flat_path, "img_preview_flat");
    }
    if let Some(darkflat_path) = get_state_param!(darkflat) {
        update_preview_from_ser_file!(builder, darkflat_path, "img_preview_darkflat");
    }
    if let Some(bias_path) = get_state_param!(bias) {
        update_preview_from_ser_file!(builder, bias_path, "img_preview_bias");
    }

    window.present();
}

fn open_ser_file<F>(title: &str, window: &ApplicationWindow, initial_file:Option<PathBuf>,callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    open_file(title, window, "video/ser", "SER", initial_file, callback);
}

fn open_toml_file<F>(title: &str, window: &ApplicationWindow, initial_file:Option<PathBuf>,callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    open_file(title, window, "application/toml", "toml", initial_file, callback);
}

fn open_file<F>(
    title: &str,
    window: &ApplicationWindow,
    mimetype: &str,
    mimename: &str,
    initial_file:Option<PathBuf>,
    callback: F,
) where
    F: Fn(PathBuf) + 'static,
{
    let initial_file = if let Some(f) = initial_file {
        gtk::gio::File::for_path(f)
    } else {
        gtk::gio::File::for_path(dirs::home_dir().unwrap())
    };


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
        .initial_file(&initial_file)
        .build();

    dialog.open(Some(window), gio::Cancellable::NONE, move |file| {
        if let Ok(file) = file {
            let filename = file.path().expect("Couldn't get file path");
            callback(filename);
        }
    });
}

fn open_folder<F>(title: &str, window: &ApplicationWindow, initial_path: Option<PathBuf>, callback: F)
where
    F: Fn(PathBuf) + 'static,
{
    println!("Folder: {:?}", initial_path);
    let initial_folder = if let Some(f) = initial_path {
        gtk::gio::File::for_path(f)
    } else {
        gtk::gio::File::for_path(dirs::home_dir().unwrap())
    };

    

    let dialog = gtk::FileDialog::builder()
        .title(title)
        .accept_label("Open")
        .modal(true)
        .initial_folder(&initial_folder)
        .build();
    dialog.set_initial_folder(Some(&initial_folder));
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





