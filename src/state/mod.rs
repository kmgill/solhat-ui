use anyhow::Result;
use gtk::glib::Sender;
use serde::{Deserialize, Serialize};
use solhat::calibrationframe::{CalibrationImage, ComputeMethod};
use solhat::context::{ProcessContext, ProcessParameters};
use solhat::drizzle::Scale;
use solhat::target::Target;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::cancel::*;
use crate::taskstatus::*;

/// Describes the parameters needed to run the SolHat algorithm
#[derive(Deserialize, Serialize, Clone)]
pub struct ParametersState {
    pub light: Option<PathBuf>,
    pub dark: Option<PathBuf>,
    pub flat: Option<PathBuf>,
    pub darkflat: Option<PathBuf>,
    pub bias: Option<PathBuf>,
    pub hot_pixel_map: Option<PathBuf>,
    pub output_dir: Option<PathBuf>,
    pub freetext: String,
    pub obs_latitude: f64,
    pub obs_longitude: f64,
    pub target: Target,
    pub obj_detection_threshold: f64,
    pub drizzle_scale: Scale,
    pub max_frames: usize,
    pub min_sigma: f64,
    pub max_sigma: f64,
    pub top_percentage: f64,
    pub decorrelated_colors: bool,
    pub analysis_window_size: usize,
    pub ld_correction: bool,
    pub ld_coefficient: f64,
    pub solar_radius_pixels: usize,
    pub vert_offset: i32,
    pub horiz_offset: i32,
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
            decorrelated_colors: false,
            analysis_window_size: 128,
            ld_correction: false,
            ld_coefficient: 0.56,
            solar_radius_pixels: 768,
            vert_offset: 0,
            horiz_offset: 0,
        }
    }
}

/// Describes the state of the UI
#[derive(Deserialize, Serialize, Default, Clone)]
pub struct UiState {
    pub last_opened_folder: Option<PathBuf>,
}

#[derive(Deserialize, Serialize, Default, Clone)]
pub struct ApplicationState {
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

    fn is_path_valid(p: &Option<PathBuf>) -> bool {
        if let Some(path) = p {
            path.exists()
        } else {
            false //  Path is None, and none is invalid.
        }
    }

    fn validate_path(p: &Option<PathBuf>) -> Option<PathBuf> {
        if ApplicationState::is_path_valid(p) {
            p.to_owned()
        } else {
            None
        }
    }

    /// Checks each path option and if the path no longer exists on the filesystem,
    /// replace it with None
    pub fn validate_paths(&mut self) {
        self.params.light = ApplicationState::validate_path(&self.params.light);
        self.params.dark = ApplicationState::validate_path(&self.params.dark);
        self.params.flat = ApplicationState::validate_path(&self.params.flat);
        self.params.darkflat = ApplicationState::validate_path(&self.params.darkflat);
        self.params.bias = ApplicationState::validate_path(&self.params.bias);
        self.params.hot_pixel_map = ApplicationState::validate_path(&self.params.hot_pixel_map);
        self.params.output_dir = ApplicationState::validate_path(&self.params.output_dir);
    }
}

lazy_static! {
    // Oh, this is such a hacky way to do it I hate it so much.
    // TODO: Learn the correct way to do this.
    pub static ref STATE: Arc<Mutex<ApplicationState>> = Arc::new(Mutex::new(ApplicationState::default()));
}

macro_rules! get_state_param {
    ($prop:ident) => {
        crate::state::STATE.lock().unwrap().params.$prop.to_owned()
    };
}

macro_rules! set_state_param {
    ($prop:ident, $value:expr) => {
        crate::state::STATE.lock().unwrap().params.$prop = $value;
    };
}

macro_rules! set_state_ui {
    ($prop:ident, $value:expr) => {
        crate::state::STATE.lock().unwrap().ui.$prop = $value;
    };
}

macro_rules! get_state_ui {
    ($prop:ident) => {
        crate::state::STATE.lock().unwrap().ui.$prop.to_owned()
    };
}

#[allow(unused_macros)]
macro_rules! clear_last_opened_folder {
    () => {
        set_state_ui!(last_opened_folder, None);
    };
}

macro_rules! get_last_opened_folder {
    () => {
        get_state_ui!(last_opened_folder)
    };
}

macro_rules! set_last_opened_folder {
    ($dir:expr) => {
        set_state_ui!(last_opened_folder, Some($dir.clone()));
        info!("Setting last opened folder to {:?}", $dir);
    };
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

pub fn build_solhat_parameters() -> Result<ProcessParameters> {
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
        analysis_window_size: state.params.analysis_window_size,
        vert_offset: state.params.vert_offset,
        horiz_offset: state.params.horiz_offset,
    })
}

pub fn build_solhat_context(sender: &Sender<TaskStatusContainer>) -> Result<ProcessContext> {
    let params = build_solhat_parameters()?;

    set_task_status(sender, "Processing Master Flat", 0, 0);
    let master_flat = if let Some(inputs) = &params.flat_inputs {
        info!("Processing master flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(sender)?;

    set_task_status(sender, "Processing Master Dark Flat", 0, 0);
    let master_darkflat = if let Some(inputs) = &params.darkflat_inputs {
        info!("Processing master dark flat...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(sender)?;

    set_task_status(sender, "Processing Master Dark", 0, 0);
    let master_dark = if let Some(inputs) = &params.dark_inputs {
        info!("Processing master dark...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(sender)?;

    set_task_status(sender, "Processing Master Bias", 0, 0);
    let master_bias = if let Some(inputs) = &params.bias_inputs {
        info!("Processing master bias...");
        CalibrationImage::new_from_file(inputs, ComputeMethod::Mean)?
    } else {
        CalibrationImage::new_empty()
    };

    check_cancel_status(sender)?;

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
