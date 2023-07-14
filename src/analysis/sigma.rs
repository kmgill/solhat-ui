use anyhow::Result;
use charts::{Chart, Color, LineSeriesView, MarkerType, ScaleLinear};
use gtk::glib::Sender;
use sciimg::{max, min};
use solhat::anaysis::frame_sigma_analysis_window_size;
use solhat::calibrationframe::CalibrationImage;
use solhat::context::ProcessContext;
use solhat::offsetting::frame_offset_analysis;
use std::sync::{Arc, Mutex};

use crate::cancel::{self, *};
use crate::state::build_solhat_parameters;
use crate::taskstatus::*;

///////////////////////////////////////////////////////
// Sigma Anaysis
///////////////////////////////////////////////////////

lazy_static! {
    // NOTE: Concurrent processing threads will stomp on each other, but at least
    // they'll do it in proper turn.  Also, this is stupid and can't stay this way.
    static ref COUNTER: Arc<Mutex<usize>> = Arc::new(Mutex::new(0));
}

#[derive(Debug)]
pub struct AnalysisRange {
    min: f64,
    max: f64,
}

#[derive(Debug)]
pub struct AnalysisSeries {
    sigma_list: Vec<f64>,
}

impl AnalysisSeries {
    pub fn sorted_list(&self) -> Vec<f64> {
        let mut sorted = self.sigma_list.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        sorted.reverse();
        sorted
    }

    pub fn minmax(&self) -> AnalysisRange {
        let mut mn = std::f64::MAX;
        let mut mx = std::f64::MIN;

        self.sigma_list.iter().for_each(|s| {
            mn = min!(*s, mn);
            mx = max!(*s, mx);
        });

        AnalysisRange { min: mn, max: mx }
    }

    pub fn sma(&self, window: usize) -> Vec<f64> {
        let half_win = window / 2;
        let mut sma: Vec<f64> = vec![];
        (0..self.sigma_list.len()).into_iter().for_each(|i| {
            let start = if i <= half_win { 0 } else { i - half_win };

            let end = if i + half_win <= self.sigma_list.len() {
                i + half_win
            } else {
                self.sigma_list.len()
            };
            let s = self.sigma_list[start..end].iter().sum::<f64>() / (end - start) as f64;
            sma.push(s);
        });
        sma
    }
}

pub fn run_sigma_analysis(
    master_sender: Sender<TaskStatusContainer>,
) -> Result<AnalysisSeries, TaskCompletion> {
    let params = match build_solhat_parameters() {
        Ok(params) => params,
        Err(why) => return Err(cancel::TaskCompletion::Error(format!("Error: {:?}", why))),
    };

    let mut context = match ProcessContext::create_with_calibration_frames(
        &params,
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
        CalibrationImage::new_empty(),
    ) {
        Ok(context) => context,
        Err(why) => return Err(cancel::TaskCompletion::Error(format!("Error: {:?}", why))),
    };

    check_cancel_status(&master_sender)?;
    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, 0);
    context.frame_records = match frame_offset_analysis(&context, move |_fr| {
        info!("frame_offset_analysis(): Frame processed.");

        let mut c = COUNTER.lock().unwrap();
        *c += 1;
        set_task_status(&sender, "Computing Center-of-Mass Offsets", frame_count, *c);
        // check_cancel_status(&sender)
    }) {
        Ok(frame_records) => frame_records,
        Err(why) => return Err(cancel::TaskCompletion::Error(format!("Error: {:?}", why))),
    };

    check_cancel_status(&master_sender)?;
    let frame_count = context.frame_records.len();
    *COUNTER.lock().unwrap() = 0;
    let sender = master_sender.clone();
    set_task_status(&sender, "Frame Sigma Analysis", frame_count, 0);
    let frame_records = match frame_sigma_analysis_window_size(
        &context,
        context.parameters.analysis_window_size,
        move |fr| {
            info!(
                "frame_sigma_analysis(): Frame processed with sigma {}",
                fr.sigma
            );

            let mut c = COUNTER.lock().unwrap();
            *c += 1;
            set_task_status(&sender, "Frame Sigma Analysis", frame_count, *c);
            // check_cancel_status(&sender)
        },
    ) {
        Ok(frame_records) => frame_records,
        Err(why) => return Err(cancel::TaskCompletion::Error(format!("Error: {:?}", why))),
    };

    let mut sigma_list: Vec<f64> = vec![];
    frame_records
        .iter()
        .filter(|fr| {
            let min_sigma = context.parameters.min_sigma.unwrap_or(std::f64::MIN);
            let max_sigma = context.parameters.max_sigma.unwrap_or(std::f64::MAX);
            fr.sigma >= min_sigma && fr.sigma <= max_sigma
        })
        .for_each(|fr| {
            sigma_list.push(fr.sigma);
        });

    set_task_completed(&master_sender);

    Ok(AnalysisSeries {
        sigma_list: sigma_list,
    })
}

// Based on https://github.com/askanium/rustplotlib/blob/master/examples/line_series_chart.rs
pub fn create_chart(data: &AnalysisSeries, width: isize, height: isize) -> Result<String> {
    let (top, right, bottom, left) = (0, 40, 50, 60);

    let x = ScaleLinear::new()
        .set_domain(vec![0_f32, data.sigma_list.len() as f32])
        .set_range(vec![0, width - left - right]);

    let rng = data.minmax();

    let y = ScaleLinear::new()
        .set_domain(vec![rng.min as f32, rng.max as f32])
        .set_range(vec![height - top - bottom, 0]);

    let line_data_1: Vec<(f32, f32)> = data
        .sorted_list()
        .iter()
        .enumerate()
        .map(|(i, s)| (i as f32, *s as f32))
        .collect();

    let line_data_2: Vec<(f32, f32)> = data
        .sma(data.sigma_list.len() / 20)
        .iter()
        .enumerate()
        .map(|(i, s)| (i as f32, *s as f32))
        .collect();

    let line_data_3: Vec<(f32, f32)> = data
        .sigma_list
        .iter()
        .enumerate()
        .map(|(i, s)| (i as f32, *s as f32))
        .collect();

    let line_view_1 = LineSeriesView::new()
        .set_x_scale(&x)
        .set_y_scale(&y)
        .set_marker_type(MarkerType::X)
        .set_label_visibility(false)
        .set_marker_visibility(false)
        .set_colors(Color::from_vec_of_hex_strings(vec!["#AAAAAA"]))
        .load_data(&line_data_1)
        .unwrap();

    let line_view_2 = LineSeriesView::new()
        .set_x_scale(&x)
        .set_y_scale(&y)
        .set_marker_type(MarkerType::X)
        .set_label_visibility(false)
        .set_marker_visibility(false)
        .set_colors(Color::from_vec_of_hex_strings(vec!["#FF4700"]))
        .load_data(&line_data_2)
        .unwrap();

    let line_view_3 = LineSeriesView::new()
        .set_x_scale(&x)
        .set_y_scale(&y)
        .set_marker_type(MarkerType::X)
        .set_label_visibility(false)
        .set_marker_visibility(false)
        .set_colors(Color::from_vec_of_hex_strings(vec!["#333333"]))
        .load_data(&line_data_3)
        .unwrap();

    // Generate and save the chart.
    let svg = Chart::new()
        .set_width(width)
        .set_height(height)
        .set_margins(top, right, bottom, left)
        .add_view(&line_view_3)
        .add_view(&line_view_2)
        .add_view(&line_view_1)
        .add_axis_bottom(&x)
        .add_axis_left(&y)
        .add_left_axis_label("Sigma Quality")
        .add_bottom_axis_label("Frame #")
        .to_string()
        .unwrap();
    Ok(svg)
}
