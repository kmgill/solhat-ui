use anyhow::Result;
use serde::{Deserialize, Serialize};
use solhat::drizzle::Scale;
use solhat::target::Target;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

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
        state::STATE.lock().unwrap().params.$prop
    };
}

macro_rules! set_state_param {
    ($prop:ident, $value:expr) => {
        state::STATE.lock().unwrap().params.$prop = $value;
    };
}

macro_rules! set_state_ui {
    ($prop:ident, $value:expr) => {
        state::STATE.lock().unwrap().ui.$prop = $value;
    };
}

#[allow(unused_macros)]
macro_rules! clear_last_opened_folder {
    () => {
        set_state_ui!(last_opened_folder, None);
    };
}

macro_rules! set_last_opened_folder {
    ($dir:expr) => {
        set_state_ui!(last_opened_folder, Some($dir.clone()));
        info!("Setting last opened folder to {:?}", $dir);
    };
}
