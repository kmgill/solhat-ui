use anyhow::{anyhow, Result};
use gtk::prelude::*;
use gtk::{glib, Application, ApplicationWindow, Builder, Button};

fn main() -> glib::ExitCode {
    let application = gtk::Application::new(Some("com.apoapsys.solhat"), Default::default());
    application.connect_activate(build_ui);
    application.run()
}

fn build_ui(application: &Application) {
    let ui_src = include_str!("../assets/solhat.ui");
    let builder = Builder::from_string(ui_src);

    let window: ApplicationWindow = builder
        .object("SolHatApplicationMain")
        .expect("Couldn't get window");
    window.set_application(Some(application));

    build_inputs_ui(&builder, &window).expect("Failed to create inputs UI");

    window.present();
}

fn build_inputs_ui(builder: &Builder, window: &ApplicationWindow) -> Result<()> {
    let btn_light_open: Button = builder
        .object("btn_light_open")
        .expect("Couldn't get button");

    btn_light_open.connect_clicked(glib::clone!(@weak window => move |_| {
        gtk::AlertDialog::builder()
            .modal(true)
            .message("Thank you for trying this example")
            .detail("You have pressed the button")
            .buttons(["Ok"])
            .build()
            .show(Some(&window));
    }));

    Ok(())
}
