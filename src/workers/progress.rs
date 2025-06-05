use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::fmt::Write;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle, ProgressState};

use crate::models::Counters;

pub struct ProgressBars {
    total_pb: ProgressBar,
    hits_pb: ProgressBar,
    stats_pb: ProgressBar,
}

impl ProgressBars {
    pub fn new(total_work: u64) -> Self {
        let multi = MultiProgress::new();
        
        // Main progress bar for overall progress with ETA
        let total_pb = multi.add(ProgressBar::new(total_work));
        total_pb.set_style(ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({percent}%) - {msg} (ETA: {eta})")
            .unwrap()
            .with_key("eta", |state: &ProgressState, w: &mut dyn Write| 
                write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
            .progress_chars("#>-"));
        total_pb.set_message("Processing phone numbers...");
        
        // Enhanced progress bar for hits with fancy emojis
        let hits_pb = multi.add(ProgressBar::new_spinner());
        hits_pb.set_style(ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] {prefix} 🎯 Hits: {pos} - {msg}")
            .unwrap());
        //hits_pb.set_prefix("[✅ HITS]");
        hits_pb.enable_steady_tick(Duration::from_millis(100));

        // Enhanced status bar for stats
        let stats_pb = multi.add(ProgressBar::new_spinner());
        stats_pb.set_style(ProgressStyle::with_template(
            "{spinner:.blue} [{elapsed_precise}] {prefix} {msg}")
            .unwrap());
        //stats_pb.set_prefix("[ℹ️ STATUS]");
        stats_pb.enable_steady_tick(Duration::from_millis(100));
        stats_pb.set_message("Starting up...");
        
        Self { total_pb, hits_pb, stats_pb }
    }
    
    pub fn update_progress(&self, completed: u64, total: Option<u64>) {
        if let Some(total) = total {
            self.total_pb.set_length(total);
        }
        self.total_pb.set_position(completed);
    }
    
    pub fn update_stats(&self, counters: &Arc<Counters>, rps: u64) {
        let success = counters.success.load(Ordering::Relaxed);
        let errors = counters.errors.load(Ordering::Relaxed);
        let ratelimits = counters.ratelimits.load(Ordering::Relaxed);
        
        self.stats_pb.set_message(format!(
            "Speed: {}/s | Success: {} | Errors: {} | Rate limits: {}",
            rps, success, errors, ratelimits
        ));
    }
    
    pub fn update_hits(&self, hits: u64, latest_hit: Option<&str>) {
        self.hits_pb.set_position(hits);
        if let Some(hit) = latest_hit {
            self.hits_pb.set_message(format!("Latest: {}", hit));
        }
    }
    
    pub fn finish(&self, hits: u64, latest_hit: Option<&str>) {
        self.total_pb.finish_with_message("✅ Processing completed!");
        self.stats_pb.finish_with_message("✅ Finished!");
        
        if hits > 1 {
            self.hits_pb.finish_with_message(format!("🎉 Found {} phone numbers! Check output.txt", hits));
        } else if hits == 1 {
            if let Some(hit) = latest_hit {
                self.hits_pb.finish_with_message(format!("🎉 Found 1 phone number! {}", hit));
            } else {
                self.hits_pb.finish_with_message("🎉 Found 1 phone number! Check output.txt");
            }
        } else {
            self.hits_pb.finish_with_message("😢 No valid phone numbers found");
        }
    }
    
    // New methods for CSV processing mode
    
    pub fn update_message(&self, message: &str) {
        self.total_pb.set_message(message.to_string());
    }
    
    pub fn reset_position(&self) {
        self.total_pb.set_position(0);
    }
    
    pub fn set_length(&self, length: u64) {
        self.total_pb.set_length(length);
    }
    
    pub fn csv_finish(&self, total_records: usize, found_records: usize) {
        self.total_pb.finish_with_message(format!("✅ Processed all {} records!", total_records));
        self.stats_pb.finish_with_message("✅ CSV processing complete!");
        
        if found_records > 0 {
            self.hits_pb.finish_with_message(format!("🎉 Found hits for {} out of {} records! Check output.csv", 
                                                    found_records, total_records));
        } else {
            self.hits_pb.finish_with_message("😢 No hits found for any records");
        }
    }
}

impl Clone for ProgressBars {
    fn clone(&self) -> Self {
        // We need to clone the progress bars properly
        let total_pb = self.total_pb.clone();
        let hits_pb = self.hits_pb.clone();
        let stats_pb = self.stats_pb.clone();
        
        Self {
            total_pb,
            hits_pb,
            stats_pb,
        }
    }
}